use std::alloc::{self, Layout};
use std::sync;

static ALLOCATOR: sync::Mutex<MachineAlloc> = sync::Mutex::new(MachineAlloc::empty());

/// A distinct allocator for interpreter memory contents, allowing us to manage its
/// memory separately from that of Miri itself. This is very useful for native-lib mode.
#[derive(Debug)]
pub struct MachineAlloc {
    /// Pointers to page-aligned memory that has been claimed by the allocator.
    /// Every pointer here must point to a page-sized allocation claimed via
    /// the global allocator.
    pages: Vec<*mut u8>,
    /// Pointers to multi-page-sized allocations. These must also be page-aligned,
    /// with their size stored as the second element of the vector.
    huge_allocs: Vec<(*mut u8, usize)>,
    /// Metadata about which bytes have been allocated on each page. The length
    /// of this vector must be the same as that of `pages`, and the length of the
    /// boxed slice must be exactly `page_size / 8`.
    ///
    /// Conceptually, each bit of the `u8` represents the allocation status of one
    /// byte on the corresponding element of `pages`; in practice, we only allocate
    /// in 8-byte chunks currently, so the `u8`s are only ever 0 (fully free) or
    /// 255 (fully allocated).
    allocated: Vec<Box<[u8]>>,
    /// The host (not emulated) page size.
    page_size: usize,
    /// If false, calls to `alloc()` and `alloc_zeroed()` just wrap the corresponding
    /// function in the global allocator. Otherwise, uses the pages tracked
    /// internally.
    enabled: bool,
}

// SAFETY: We only point to heap-allocated data
unsafe impl Send for MachineAlloc {}

impl MachineAlloc {
    /// Initializes the allocator. `page_size` is set to 4k as a placeholder to
    /// allow this function to be `const`; it is updated to its real value when
    /// `enable()` is called.
    const fn empty() -> Self {
        Self {
            pages: Vec::new(),
            huge_allocs: Vec::new(),
            allocated: Vec::new(),
            page_size: 4096,
            enabled: true,
        }
    }

    /// Enables the allocator. From this point onwards, calls to `alloc()` and
    /// `alloc_zeroed()` will return `(ptr, false)`.
    pub fn enable() {
        let mut alloc = ALLOCATOR.lock().unwrap();
        alloc.enabled = true;
        // This needs to specifically be the system pagesize!
        alloc.page_size = unsafe {
            // If sysconf errors, better to just panic
            libc::sysconf(libc::_SC_PAGE_SIZE).try_into().unwrap()
        }
    }

    /// Expands the available memory pool by adding one page.
    fn add_page(&mut self) {
        let page_layout =
            unsafe { Layout::from_size_align_unchecked(self.page_size, self.page_size) };
        let page_ptr = unsafe { alloc::alloc(page_layout) };
        self.allocated.push(vec![0u8; self.page_size / 8].into_boxed_slice());
        self.pages.push(page_ptr);
    }

    /// For simplicity, we allocate in multiples of 8 bytes with at least that
    /// alignment.
    #[inline]
    fn normalized_layout(layout: Layout) -> (usize, usize) {
        let align = if layout.align() < 8 { 8 } else { layout.align() };
        let size = layout.size().next_multiple_of(8);
        (size, align)
    }

    /// Allocates memory as described in `Layout`. If `MachineAlloc::enable()`
    /// has *not* been called yet, this is just a wrapper for `(alloc::alloc(),
    /// true)`. Otherwise, it will allocate from its own memory pool and
    /// return `(ptr, false)`. The latter field is meant to correspond with the
    /// field `alloc_is_global` for `MiriAllocBytes`.
    ///
    /// SAFETY: See alloc::alloc()
    #[inline]
    pub unsafe fn alloc(layout: Layout) -> (*mut u8, bool) {
        let mut alloc = ALLOCATOR.lock().unwrap();
        unsafe {
            if alloc.enabled {
                (alloc.alloc_inner(layout, alloc::alloc), false)
            } else {
                (alloc::alloc(layout), true)
            }
        }
    }

    /// Same as `alloc()`, but zeroes out data before allocating. Instead
    /// wraps `alloc::alloc_zeroed()` if `MachineAlloc::enable()` has not been
    /// called yet.
    ///
    /// SAFETY: See alloc::alloc_zeroed()
    pub unsafe fn alloc_zeroed(layout: Layout) -> (*mut u8, bool) {
        let mut alloc = ALLOCATOR.lock().unwrap();
        if alloc.enabled {
            let ptr = unsafe { alloc.alloc_inner(layout, alloc::alloc_zeroed) };
            (ptr, false)
        } else {
            unsafe { (alloc::alloc_zeroed(layout), true) }
        }
    }

    /// SAFETY: The allocator must have been `enable()`d already and
    /// the `layout` must be valid.
    unsafe fn alloc_inner(&mut self, layout: Layout, sys_allocator: unsafe fn(Layout) -> *mut u8) -> *mut u8 {
        let (size, align) = MachineAlloc::normalized_layout(layout);

        if align > self.page_size || size > self.page_size {
            unsafe {
                self.alloc_multi_page(layout, sys_allocator)
            }
        } else {
            for (page, pinfo) in std::iter::zip(&mut self.pages, &mut self.allocated) {
                for idx in (0..self.page_size).step_by(align) {
                    let idx_pinfo = idx / 8;
                    let size_pinfo = size / 8;
                    if pinfo.len() < idx_pinfo + size_pinfo {
                        break;
                    }
                    if pinfo[idx_pinfo..idx_pinfo + size_pinfo].iter().all(|v| *v == 0) {
                        pinfo[idx_pinfo..idx_pinfo + size_pinfo].fill(255);
                        unsafe {
                            let ret = page.offset(idx.try_into().unwrap());
                            if ret.addr() >= page.addr() + self.page_size {
                                panic!("Returing {} from page {}", ret.addr(), page.addr());
                            }
                            return page.offset(idx.try_into().unwrap());
                        }
                    }
                }
            }

            // We get here only if there's no space in our existing pages
            self.add_page();
            unsafe { self.alloc_inner(layout, sys_allocator) }
        }
    }

    /// SAFETY: Same as `alloc_inner()` with the added requirement that `layout`
    /// must ask for a size larger than the host pagesize.
    unsafe fn alloc_multi_page(&mut self, layout: Layout, sys_allocator: unsafe fn(Layout) -> *mut u8) -> *mut u8 {
        let ret = unsafe { sys_allocator(layout) };
        self.huge_allocs.push((ret, layout.size()));
        ret
    }

    /// Deallocates a pointer from the machine allocator. While not unsound,
    /// attempting to deallocate a pointer if `MachineAlloc` has not been enabled
    /// will likely result in a panic.
    ///
    /// SAFETY: This pointer must have been allocated with `MachineAlloc::alloc()`
    /// (or `alloc_zeroed()`) which must have returned `(ptr, false)` specifically!
    /// If it returned `(ptr, true)`, then deallocate it with `alloc::dealloc()` instead.
    pub unsafe fn dealloc(ptr: *mut u8, layout: Layout) {
        let mut alloc_guard = ALLOCATOR.lock().unwrap();
        // Doing it this way lets us grab 2 mutable references to different fields at once
        let alloc: &mut MachineAlloc = &mut alloc_guard;

        let (size, align) = MachineAlloc::normalized_layout(layout);

        if size == 0 || ptr.is_null() {
            return;
        }

        let ptr_idx = ptr.addr() % alloc.page_size;
        let page_addr = ptr.addr() - ptr_idx;

        if align > alloc.page_size || size > alloc.page_size {
            unsafe {
                alloc.dealloc_multi_page(ptr, layout);
            }
        } else {
            let pinfo = std::iter::zip(&mut alloc.pages, &mut alloc.allocated)
                .find(|(page, _)| page.addr() == page_addr);
            let Some((_, pinfo)) = pinfo else {
                panic!("Freeing in an unallocated page: {ptr:?}\nHolding pages {:?}", alloc.pages)
            };
            let ptr_idx_pinfo = ptr_idx / 8;
            let size_pinfo = size / 8;
            // Everything is always aligned to at least 8 bytes so this is ok
            pinfo[ptr_idx_pinfo..ptr_idx_pinfo + size_pinfo].fill(0);
        }

        let mut free = vec![];
        let page_layout =
            unsafe { Layout::from_size_align_unchecked(alloc.page_size, alloc.page_size) };
        for (idx, pinfo) in alloc.allocated.iter().enumerate() {
            if pinfo.iter().all(|p| *p == 0) {
                free.push(idx);
            }
        }
        free.reverse();
        for idx in free {
            let _ = alloc.allocated.remove(idx);
            unsafe {
                alloc::dealloc(alloc.pages.remove(idx), page_layout);
            }
        }
    }

    /// SAFETY: Same as `dealloc()` with the added requirement that `layout`
    /// must ask for a size larger than the host pagesize.
    unsafe fn dealloc_multi_page(&mut self, ptr: *mut u8, layout: Layout) {
        let (idx, _) = self
            .huge_allocs
            .iter()
            .enumerate()
            .find(|pg| ptr.addr() == pg.1.0.addr())
            .expect("Freeing unallocated pages");
        let ptr = self.huge_allocs.remove(idx).0;
        unsafe {
            alloc::dealloc(ptr, layout);
        }
    }
}
