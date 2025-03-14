use genmc_sys::{GENMC_MAIN_THREAD_ID, GenmcThreadId};
use rustc_data_structures::fx::FxHashMap;

use crate::ThreadId;

#[derive(Debug)]
pub struct ThreadInfo {
    pub miri_tid: ThreadId,
    pub genmc_tid: GenmcThreadId,
    // TODO GENMC: Do we need this? Only for the main thread?
    pub user_code_finished: bool,
}

impl ThreadInfo {
    const MAIN_THREAD_INFO: Self = Self::new(ThreadId::MAIN_THREAD, GENMC_MAIN_THREAD_ID);

    #[must_use]
    pub const fn new(miri_tid: ThreadId, genmc_tid: GenmcThreadId) -> Self {
        Self { miri_tid, genmc_tid, user_code_finished: false }
    }
}

#[derive(Debug)]
pub struct ThreadInfoManager {
    tid_map: FxHashMap<ThreadId, GenmcThreadId>,
    thread_infos: Vec<ThreadInfo>,
}

impl Default for ThreadInfoManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ThreadInfoManager {
    #[must_use]
    pub fn new() -> Self {
        let mut tid_map = FxHashMap::default();
        tid_map.insert(ThreadId::MAIN_THREAD, GENMC_MAIN_THREAD_ID);
        let thread_infos = vec![ThreadInfo::MAIN_THREAD_INFO];
        Self { tid_map, thread_infos }
    }

    pub fn reset(&mut self) {
        self.tid_map.clear();
        self.tid_map.insert(ThreadId::MAIN_THREAD, GENMC_MAIN_THREAD_ID);
        self.thread_infos.clear();
        self.thread_infos.push(ThreadInfo::MAIN_THREAD_INFO);
    }

    #[must_use]
    #[allow(unused)]
    pub fn thread_count(&self) -> usize {
        self.thread_infos.len()
    }

    pub fn add_thread(&mut self, thread_id: ThreadId) -> GenmcThreadId {
        // NOTE: GenMC thread ids are integers incremented by one every time
        let index = self.thread_infos.len();
        let genmc_tid = GenmcThreadId(index.try_into().unwrap());
        let thread_info = ThreadInfo::new(thread_id, genmc_tid);
        // TODO GENMC: Document this in place where ThreadIds are created
        assert!(
            self.tid_map.insert(thread_id, genmc_tid).is_none(),
            "Cannot reuse thread ids: thread id {thread_id:?} already inserted"
        );
        self.thread_infos.push(thread_info);

        genmc_tid
    }

    #[must_use]
    pub fn get_info(&self, thread_id: ThreadId) -> &ThreadInfo {
        let genmc_tid = *self.tid_map.get(&thread_id).unwrap();
        self.get_info_genmc(genmc_tid)
    }

    #[must_use]
    pub fn get_info_genmc(&self, genmc_tid: GenmcThreadId) -> &ThreadInfo {
        let index: usize = genmc_tid.0.try_into().unwrap();
        &self.thread_infos[index]
    }

    #[must_use]
    pub fn get_info_mut(&mut self, thread_id: ThreadId) -> &mut ThreadInfo {
        let genmc_tid = *self.tid_map.get(&thread_id).unwrap();
        self.get_info_mut_genmc(genmc_tid)
    }

    #[must_use]
    pub fn get_info_mut_genmc(&mut self, genmc_tid: GenmcThreadId) -> &mut ThreadInfo {
        let index: usize = genmc_tid.0.try_into().unwrap();
        &mut self.thread_infos[index]
    }
}
