use std::alloc::{self, Layout};
use std::sync;

use nix::sys::mman::ProtFlags;

static ALLOCATOR: sync::Mutex<MachineAlloc> = sync::Mutex::new(MachineAlloc::empty());

// FIXME: Assume 4k pages for now, needs adjusting

/// A distinct allocator for the `MiriMachine`, allowing us to manage its
/// memory separately from that of Miri itself.
#[derive(Debug)]
pub struct MachineAlloc {
    pages: Vec<*mut u8>,
    huge_allocs: Vec<(*mut u8, usize)>,
    allocated: Vec<Box<[u8]>>,
    page_size: usize,
    enabled: bool,
    ffi_mode: bool,
}

// SAFETY: We only point to heap-allocated data
unsafe impl Send for MachineAlloc {}

impl MachineAlloc {
    // Allocation-related methods

    /// Initializes the allocator with 4k pages.
    const fn empty() -> Self {
        Self {
            pages: Vec::new(),
            huge_allocs: Vec::new(),
            allocated: Vec::new(),
            page_size: 4096,
            //taken_sigaction: None,
            enabled: false,
            ffi_mode: false,
        }
    }

    /// SAFETY: There must be no existing `MiriAllocBytes`
    pub unsafe fn enable() {
        let mut alloc = ALLOCATOR.lock().unwrap();
        alloc.enabled = true;
    }

    /// SAFETY: `MachineAlloc` must be empty.
    #[allow(clippy::cast_possible_truncation)]
    pub unsafe fn set_pagesize(page_size: u64) {
        let mut alloc = ALLOCATOR.lock().unwrap();
        assert!(page_size == 4096 || page_size == 4096 * 4);
        alloc.page_size = page_size as usize;
    }

    /// Returns a vector of page addresses managed by the allocator.
    pub fn pages() -> Vec<usize> {
        let alloc = ALLOCATOR.lock().unwrap();
        alloc.pages.clone().into_iter().map(|p| p.addr()).collect()
    }

    fn add_page(&mut self) {
        let page_layout =
            unsafe { Layout::from_size_align_unchecked(self.page_size, self.page_size) };
        let page_ptr = unsafe { alloc::alloc(page_layout) };
        if page_ptr.is_null() {
            panic!("aligned_alloc failed!!!")
        }
        self.allocated.push(vec![0u8; self.page_size / 8].into_boxed_slice());
        self.pages.push(page_ptr);
    }

    #[inline]
    fn normalized_layout(layout: Layout) -> (usize, usize) {
        let align = if layout.align() < 8 { 8 } else { layout.align() };
        let size = layout.size().next_multiple_of(8);
        (size, align)
    }

    #[inline]
    fn huge_normalized_layout(&self, layout: Layout) -> (usize, usize) {
        let size = layout.size().next_multiple_of(self.page_size);
        let align = std::cmp::max(layout.align(), 4096);
        (size, align)
    }

    /// SAFETY: See alloc::alloc()
    #[inline]
    pub unsafe fn alloc(layout: Layout) -> *mut u8 {
        let mut alloc = ALLOCATOR.lock().unwrap();
        unsafe { if alloc.enabled { alloc.alloc_inner(layout) } else { alloc::alloc(layout) } }
    }

    /// SAFETY: See alloc::alloc_zeroed()
    pub unsafe fn alloc_zeroed(layout: Layout) -> *mut u8 {
        let mut alloc = ALLOCATOR.lock().unwrap();
        if alloc.enabled {
            let ptr = unsafe { alloc.alloc_inner(layout) };
            if !ptr.is_null() {
                unsafe {
                    ptr.write_bytes(0, layout.size());
                }
            }
            ptr
        } else {
            unsafe { alloc::alloc_zeroed(layout) }
        }
    }

    /// SAFETY: See alloc::alloc()
    #[allow(clippy::cast_possible_wrap)]
    unsafe fn alloc_inner(&mut self, layout: Layout) -> *mut u8 {
        let (size, align) = MachineAlloc::normalized_layout(layout);

        if align > self.page_size || size > self.page_size {
            unsafe { self.alloc_multi_page(layout) }
        } else {
            for (page, pinfo) in std::iter::zip(&mut self.pages, &mut self.allocated) {
                for idx in (0..self.page_size).step_by(align) {
                    if pinfo.len() < idx / 8 + size / 8 {
                        break;
                    }
                    if pinfo[idx / 8..idx / 8 + size / 8].iter().all(|v| *v == 0) {
                        pinfo[idx / 8..idx / 8 + size / 8].fill(255);
                        unsafe {
                            let ret = page.offset(idx as isize);
                            if ret.addr() >= page.addr() + self.page_size {
                                panic!("Returing {} from page {}", ret.addr(), page.addr());
                            }
                            return page.offset(idx as isize);
                        }
                    }
                }
            }

            // We get here only if there's no space in our existing pages
            self.add_page();
            unsafe { self.alloc_inner(layout) }
        }
    }

    /// SAFETY: See alloc::alloc()
    unsafe fn alloc_multi_page(&mut self, layout: Layout) -> *mut u8 {
        let (size, align) = self.huge_normalized_layout(layout);

        let layout = unsafe { Layout::from_size_align_unchecked(size, align) };
        let ret = unsafe { alloc::alloc(layout) };
        self.huge_allocs.push((ret, size));
        ret
    }

    /// Safety: see alloc::dealloc()
    pub unsafe fn dealloc(ptr: *mut u8, layout: Layout) {
        let mut alloc = ALLOCATOR.lock().unwrap();
        unsafe {
            if alloc.enabled {
                alloc.dealloc_inner(ptr, layout);
            } else {
                alloc::dealloc(ptr, layout);
            }
        }
    }

    /// SAFETY: See alloc::dealloc()
    unsafe fn dealloc_inner(&mut self, ptr: *mut u8, layout: Layout) {
        let (size, align) = MachineAlloc::normalized_layout(layout);

        if size == 0 || ptr.is_null() {
            return;
        }

        let ptr_idx = ptr.addr() % self.page_size;
        let page_addr = ptr.addr() - ptr_idx;

        if align > self.page_size || size > self.page_size {
            unsafe {
                self.dealloc_multi_page(ptr, layout);
            }
        } else {
            let pinfo = std::iter::zip(&mut self.pages, &mut self.allocated).find(|(page, _)| {
                page.addr() == page_addr
            });
            let Some((_, pinfo)) = pinfo else {
                panic!("Freeing in an unallocated page: {ptr:?}\nHolding pages {:?}", self.pages)
            };

            // Everything is always aligned to at least 8 bytes so this is ok
            pinfo[ptr_idx / 8..ptr_idx / 8 + size / 8].fill(0);
        }

        let mut free = vec![];
        let page_layout =
            unsafe { Layout::from_size_align_unchecked(self.page_size, self.page_size) };
        for (idx, pinfo) in self.allocated.iter().enumerate() {
            if pinfo.iter().all(|p| *p == 0) {
                free.push(idx);
            }
        }
        free.reverse();
        for idx in free {
            let _ = self.allocated.remove(idx);
            unsafe {
                alloc::dealloc(self.pages.remove(idx), page_layout);
            }
        }
    }

    /// SAFETY: See alloc::dealloc()
    unsafe fn dealloc_multi_page(&mut self, ptr: *mut u8, layout: Layout) {
        let (idx, _) = self
            .huge_allocs
            .iter()
            .enumerate()
            .find(|pg| ptr.addr() == pg.1.0.addr())
            .expect("Freeing unallocated pages");
        let ptr = self.huge_allocs.remove(idx).0;
        let (size, align) = self.huge_normalized_layout(layout);
        unsafe {
            let layout = Layout::from_size_align_unchecked(size, align);
            alloc::dealloc(ptr, layout);
        }
    }

    // Protection-related methods

    /// Protects all owned memory, preventing accesses.
    ///
    /// SAFETY: Accessing memory after this point will result in a segfault
    /// unless it is first unprotected.
    pub unsafe fn prepare_ffi() -> Option<()> {
        let mut alloc = ALLOCATOR.lock().unwrap();
        unsafe {
            alloc.mprotect(ProtFlags::PROT_NONE).unwrap();
        }
        alloc.ffi_mode = true;
        Some(())
    }

    /// Deprotects all owned memory by setting it to RW.
    pub fn unprep_ffi() -> Option<()> {
        let mut alloc = ALLOCATOR.lock().unwrap();
        let default_flags = ProtFlags::PROT_READ | ProtFlags::PROT_WRITE;
        unsafe {
            alloc.mprotect(default_flags)?;
        }
        alloc.ffi_mode = false;
        Some(())
    }

    /// Applies `prot` to every page managed by the allocator.
    ///
    /// SAFETY: Accessing memory in violation of the protection flags will
    /// trigger a segfault.
    unsafe fn mprotect(&mut self, prot: ProtFlags) -> Option<()> {
        for &pg in &self.pages {
            unsafe {
                let addr: std::ptr::NonNull<std::ffi::c_void> =
                    std::ptr::NonNull::new_unchecked(pg as *mut _);
                nix::sys::mman::mprotect(addr, self.page_size, prot).ok()?;
            }
        }
        for &(hpg, size) in &self.huge_allocs {
            unsafe {
                let addr = std::ptr::NonNull::new_unchecked(hpg as *mut _);
                nix::sys::mman::mprotect(addr, size, prot).ok()?;
            }
        }
        Some(())
    }
}

// TODO: Add tests for... all of this?
