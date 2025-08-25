use genmc_sys::GENMC_MAIN_THREAD_ID;
use rustc_data_structures::fx::FxHashMap;

use crate::ThreadId;

#[derive(Debug)]
pub struct ThreadIdMap {
    /// Map from Miri thread IDs to GenMC thread IDs.
    /// We assume as little as possible about Miri thread IDs, so we use a map.
    miri_to_genmc: FxHashMap<ThreadId, i32>,
    /// Map from GenMC thread IDs to Miri thread IDs.
    /// We control which thread IDs are used, so we choose them in as an incrementing counter.
    genmc_to_miri: Vec<ThreadId>, // FIXME(genmc): check if this assumption is (and will stay) correct.
}

impl Default for ThreadIdMap {
    fn default() -> Self {
        let miri_to_genmc = [(ThreadId::MAIN_THREAD, GENMC_MAIN_THREAD_ID)].into_iter().collect();
        let genmc_to_miri = vec![ThreadId::MAIN_THREAD];
        Self { miri_to_genmc, genmc_to_miri }
    }
}

impl ThreadIdMap {
    pub fn reset(&mut self) {
        self.miri_to_genmc.clear();
        self.miri_to_genmc.insert(ThreadId::MAIN_THREAD, GENMC_MAIN_THREAD_ID);
        self.genmc_to_miri.clear();
        self.genmc_to_miri.push(ThreadId::MAIN_THREAD);
    }

    #[must_use]
    /// Add a new Miri thread to the mapping and dispense a new thread ID for GenMC to use.
    pub fn add_thread(&mut self, thread_id: ThreadId) -> i32 {
        // NOTE: We select the new thread ids as integers incremented by one (we use the length as the counter).
        let next_thread_id = self.genmc_to_miri.len();
        let genmc_tid = next_thread_id.try_into().unwrap();
        // FIXME(genmc): Fix this, or document this where ThreadIds are created (where is this?)
        if self.miri_to_genmc.insert(thread_id, genmc_tid).is_some() {
            panic!("Cannot reuse thread ids: thread id {thread_id:?} already inserted.");
        }
        self.genmc_to_miri.push(thread_id);

        genmc_tid
    }

    #[must_use]
    /// Try to get the GenMC thread ID corresponding to a given Miri `ThreadId`.
    /// Panics if there is no mapping for the given `ThreadId`.
    pub fn get_genmc_tid(&self, thread_id: ThreadId) -> i32 {
        *self.miri_to_genmc.get(&thread_id).unwrap()
    }

    #[must_use]
    /// Try to get the Miri `ThreadId` corresponding to a given GenMC thread id.
    pub fn try_get_miri_tid(&self, genmc_tid: impl TryInto<i32>) -> Option<ThreadId> {
        let genmc_tid: i32 = genmc_tid.try_into().ok()?;
        let index: usize = genmc_tid.try_into().ok()?;
        self.genmc_to_miri.get(index).cloned()
    }
}
