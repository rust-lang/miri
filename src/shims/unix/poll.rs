use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::time::Duration;

use crate::shims::files::FdNum;
use crate::shims::{DynFileDescriptionRef, FdId};
use crate::*;

/// An interest into a file descriptor together with its
/// relevant readiness events.
struct PollInterest {
    /// The file descriptor for which this interest is for.
    fd_num: FdNum,
    /// The file description to which the file descriptor belongs.
    fd: DynFileDescriptionRef,
    /// The readiness events this interest is interested in.
    relevant_events: Readiness,
}

/// Struct used for receiving readiness event updates from the
/// readiness manager.
struct Poll {
    /// The thread which is blocked by this poll instance.
    thread: Cell<ThreadId>,
    /// The readiness interests of this poll instance.
    interests: Vec<PollInterest>,
}

impl ReadinessConsumer for Rc<Poll> {
    fn ready_event<'tcx>(
        &self,
        fd_id: FdId,
        readiness: Readiness,
        _force_edge: bool,
        ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx> {
        let is_any_fulfilled =
            self.interests.iter().filter(|interest| interest.fd.id() == fd_id).any(|interest| {
                interest.relevant_events.as_ref() & readiness.as_ref() != Readiness::EMPTY
            });

        if is_any_fulfilled {
            ecx.unblock_thread(self.thread.get(), BlockReason::Poll)?;
        }

        interp_ok(())
    }
}

impl VisitProvenance for Poll {
    fn visit_provenance(&self, _visit: &mut VisitWith<'_>) {}
}

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    fn poll(
        &mut self,
        fds: &OpTy<'tcx>,
        nfds: &OpTy<'tcx>,
        timeout: &OpTy<'tcx>,
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();

        let nfds_layout = this.libc_ty_layout("nfds_t");
        let nfds: u64 = this.read_scalar(nfds)?.to_int(nfds_layout.size)?.try_into().unwrap();
        let timeout = this.read_scalar(timeout)?.to_i32()?;

        let deadline = if timeout.is_positive() {
            let timeout_duration = Duration::from_millis(u64::try_from(timeout).unwrap());
            Some(this.machine.monotonic_clock.now().add_lossy(timeout_duration).into())
        } else {
            None
        };

        let fds_arr_layout = this.libc_array_ty_layout("pollfd", nfds);
        let fds_arr_mplace = this.deref_pointer_as(fds, fds_arr_layout)?;
        let mut fds_arr_iter = this.project_array_fields(&fds_arr_mplace)?;

        let mut interests = Vec::new();
        let mut revents = BTreeMap::new();
        let mut fulfilled_interests = 0u32;

        // We iterate over the fds array of the `poll` syscall. For each fd, we check its
        // output field, the relevant events, and whether they are currently fulfilled.
        while let Some((_idx, pollfd)) = fds_arr_iter.next(this)? {
            let fd_field = this.project_field_named(&pollfd, "fd")?;
            let fd_num = this.read_scalar(&fd_field)?.to_i32()?;
            let Some(fd) = this.machine.fds.get(fd_num) else {
                return this.set_errno_and_return_neg1(LibcError("EBADF"), dest);
            };

            let events_field = this.project_field_named(&pollfd, "events")?;
            let events = this.read_scalar(&events_field)?.to_u16()?;

            let revents_field = this.project_field_named(&pollfd, "revents")?;

            let relevant_events = this.poll_bitflag_to_readiness(events)?;
            let active_events = relevant_events.as_ref() & fd.readiness()?.as_ref();
            if active_events != Readiness::EMPTY {
                // The interest in this file description is currently fulfilled.
                fulfilled_interests = fulfilled_interests.strict_add(1);
                let poll_events = this.readiness_to_poll_bitflag(&active_events);
                this.write_scalar(Scalar::from_u16(poll_events), &revents_field)?;
            } else {
                // The interest in this file description is currently not fulfilled.
                // Since we later only update the `revents` field for FDs which receive
                // an event, we initially zero this field.
                this.write_null(&revents_field)?;
            }

            interests.push(PollInterest { fd, fd_num, relevant_events });
            revents.insert(fd_num, revents_field.clone());
        }

        if fulfilled_interests > 0 {
            // Some interests are already fulfilled. We thus don't need to
            // create a `Poll` instance and add it to the readiness manager,
            // and can just return here.
            return this.write_scalar(Scalar::from_u32(fulfilled_interests), dest);
        }

        // None of the interests are currently fulfilled.
        // We create a `Poll` instance and add it to the readiness manager
        // to get notified about readiness changes for our interested FDs.

        let poll =
            Rc::new(Poll { interests, thread: Cell::new(this.machine.threads.active_thread()) });
        let readiness_consumer_id = this.machine.readiness.register_consumer(poll.clone());
        poll.interests
            .iter()
            .map(|interest| interest.fd.id())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .for_each(|fd| this.machine.readiness.register_interest(fd, readiness_consumer_id));

        let dest = dest.clone();
        this.block_thread(
            BlockReason::Poll,
            deadline,
            callback!(
                @capture<'tcx> {
                    readiness_consumer_id: ReadinessConsumerId,
                    poll: Rc<Poll>,
                    revents: BTreeMap<FdNum, MPlaceTy<'tcx>>,
                    dest: MPlaceTy<'tcx>,
                } |this, _reason: UnblockKind| {
                    // Ensure the `Poll` instance no longer receives any ready events
                    // which would cause duplicate thread unblocks.
                    this.machine.readiness.deregister_consumer(readiness_consumer_id);

                    let mut fulfilled_interests = 0u32;

                    for interest in &poll.interests {
                        let active_events = interest.relevant_events.as_ref() & interest.fd.readiness()?.as_ref();
                        if active_events != Readiness::EMPTY {
                            // The interest in this file description is fulfilled.
                            fulfilled_interests = fulfilled_interests.strict_add(1);
                            let poll_events = this.readiness_to_poll_bitflag(&active_events);
                            let revents_place = revents.get(&interest.fd_num).unwrap();
                            this.write_scalar(Scalar::from_u16(poll_events), revents_place)?;
                        }
                    }

                    this.write_scalar(Scalar::from_u32(fulfilled_interests), &dest)
                }
            ),
        );

        interp_ok(())
    }
}

impl<'tcx> VisitProvenance for BTreeMap<FdNum, MPlaceTy<'tcx>> {
    fn visit_provenance(&self, visit: &mut VisitWith<'_>) {
        self.values().for_each(|place| place.visit_provenance(visit));
    }
}

impl<'tcx> EvalContextPrivExt<'tcx> for crate::MiriInterpCx<'tcx> {}
trait EvalContextPrivExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    /// Convert a [`Readiness`] instance into the corresponding poll
    /// readiness bitflag.
    fn readiness_to_poll_bitflag(&self, readiness: &Readiness) -> u16 {
        let this = self.eval_context_ref();

        let pollin = this.eval_libc_u16("POLLIN");
        let pollout = this.eval_libc_u16("POLLOUT");
        let pollrdhup = this.eval_libc_u16("POLLRDHUP");
        let pollhup = this.eval_libc_u16("POLLHUP");
        let pollerr = this.eval_libc_u16("POLLERR");

        let mut bitflag = 0;
        if readiness.readable {
            bitflag |= pollin;
        }
        if readiness.writable {
            bitflag |= pollout;
        }
        if readiness.read_closed {
            bitflag |= pollrdhup;
        }
        if readiness.write_closed {
            bitflag |= pollhup;
        }
        if readiness.error {
            bitflag |= pollerr;
        }

        bitflag
    }

    /// Convert a poll readiness bitflag into the corresponding
    /// [`Readiness`] instance.
    fn poll_bitflag_to_readiness(&self, mut bitflag: u16) -> InterpResult<'tcx, Readiness> {
        let this = self.eval_context_ref();

        let pollin = this.eval_libc_u16("POLLIN");
        let pollout = this.eval_libc_u16("POLLOUT");
        let pollrdhup = this.eval_libc_u16("POLLRDHUP");
        let pollhup = this.eval_libc_u16("POLLHUP");
        let pollerr = this.eval_libc_u16("POLLERR");

        let mut readiness = Readiness::EMPTY;
        if bitflag & pollin == pollin {
            readiness.readable = true;
            bitflag &= !pollin;
        }
        if bitflag & pollout == pollout {
            readiness.writable = true;
            bitflag &= !pollout;
        }
        if bitflag & pollrdhup == pollrdhup {
            readiness.read_closed = true;
            bitflag &= !pollrdhup;
        }
        if bitflag & pollhup == pollhup {
            readiness.write_closed = true;
            bitflag &= !pollhup;
        }
        if bitflag & pollerr == pollerr {
            readiness.error = true;
            bitflag &= !pollerr;
        }

        if bitflag != 0 {
            throw_unsup_format!(
                "poll: poll event {bitflag:#x} is unsupported. Only POLLIN, \
                POLLOUT, POLLERR, POLLHUP, and POLLRDHUP are supported."
            );
        }

        interp_ok(readiness)
    }
}
