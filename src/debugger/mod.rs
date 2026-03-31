pub mod channel;
mod state;
pub mod tui;

use std::cell::RefCell;

use self::channel::{CommandReceiver, StateSender};

pub use self::state::DebuggerState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DebuggerCommand {
	Continue,
	StepOver,
	StepBack,
	RunToFrame(String),
	RunToMain,
	RunToEnd,
	Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DebuggerMode {
	Step,
	Continue,
	RunToFrame(String),
	RunToMain,
	RunToEnd,
}

pub struct MiriDebuggerHandle {
	state_tx: StateSender,
	cmd_rx: CommandReceiver,
	mode: RefCell<DebuggerMode>,
}

impl MiriDebuggerHandle {
	pub fn new(state_tx: StateSender, cmd_rx: CommandReceiver) -> Self {
		Self { state_tx, cmd_rx, mode: RefCell::new(DebuggerMode::Step) }
	}

	pub fn send(&self, state: DebuggerState) {
		let current_mode = self.mode.borrow().clone();
		match current_mode {
			DebuggerMode::Step => {
				let _ = self.state_tx.send(state);
			}
			DebuggerMode::Continue => {}
			DebuggerMode::RunToFrame(ref target) => {
				let target_lc = target.to_ascii_lowercase();
				if state
					.stack_frames
					.iter()
					.any(|frame| frame.fn_name.to_ascii_lowercase().contains(&target_lc))
				{
					*self.mode.borrow_mut() = DebuggerMode::Step;
				}
				let _ = self.state_tx.send(state);
			}
			DebuggerMode::RunToMain => {
				if state.in_user_code {
					*self.mode.borrow_mut() = DebuggerMode::Step;
				}
				let _ = self.state_tx.send(state);
			}
			DebuggerMode::RunToEnd => {
				let _ = self.state_tx.send(state);
			}
		}
	}

	pub fn wait_for_continue(&self) -> DebuggerCommand {
		if !matches!(&*self.mode.borrow(), DebuggerMode::Step) {
			return DebuggerCommand::Continue;
		}

		match self.cmd_rx.recv().unwrap_or(DebuggerCommand::Continue) {
			DebuggerCommand::Continue => {
				*self.mode.borrow_mut() = DebuggerMode::Continue;
				DebuggerCommand::Continue
			}
			DebuggerCommand::StepOver => {
				*self.mode.borrow_mut() = DebuggerMode::Step;
				DebuggerCommand::StepOver
			}
			DebuggerCommand::StepBack => {
				// Reverse stepping is handled entirely in the TUI thread.
				DebuggerCommand::Continue
			}
			DebuggerCommand::RunToFrame(target) => {
				*self.mode.borrow_mut() = DebuggerMode::RunToFrame(target);
				DebuggerCommand::Continue
			}
			DebuggerCommand::RunToMain => {
				*self.mode.borrow_mut() = DebuggerMode::RunToMain;
				DebuggerCommand::Continue
			}
			DebuggerCommand::RunToEnd => {
				*self.mode.borrow_mut() = DebuggerMode::RunToEnd;
				DebuggerCommand::Continue
			}
			DebuggerCommand::Quit => DebuggerCommand::Quit,
		}
	}
}
