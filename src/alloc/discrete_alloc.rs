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
}

// SAFETY: We only point to heap-allocated data
unsafe impl Send for MachineAlloc {}

impl MachineAlloc {
    /// Initializes the allocator. `page_size` is set to 0 as a placeholder to
    /// allow this function to be `const`; it is updated to its real value on
    /// the first call to `alloc()` or `alloc_zeroed()`.
    const fn empty() -> Self {
        Self {
            pages: Vec::new(),
            huge_allocs: Vec::new(),
            allocated: Vec::new(),
            page_size: 0,
        }
    }

    /// Expands the available memory pool by adding one page.
    fn add_page(&mut self) {
        let page_layout =
            unsafe { Layout::from_size_align_unchecked(self.page_size, self.page_size) };
        // We don't overwrite the bytes we hand out so make sure they're zeroed by default!
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

    /// Allocates memory as described in `Layout`.
    ///
    /// SAFETY: `See alloc::alloc()`
    #[inline]
    pub unsafe fn alloc(layout: Layout) -> *mut u8 {
        let mut alloc = ALLOCATOR.lock().unwrap();
        unsafe {
            alloc.alloc_inner(layout, false)
        }
    }

    /// Same as `alloc()`, but zeroes out data before allocating.
    ///
    /// SAFETY: See `alloc::alloc_zeroed()`
    pub unsafe fn alloc_zeroed(layout: Layout) -> *mut u8 {
        let mut alloc = ALLOCATOR.lock().unwrap();
        unsafe { alloc.alloc_inner(layout, true) }
    }

    /// SAFETY: See `alloc::alloc()`
    unsafe fn alloc_inner(&mut self, layout: Layout, zeroed: bool) -> *mut u8 {
        let (size, align) = MachineAlloc::normalized_layout(layout);

        if self.page_size == 0 {
            unsafe {
                self.page_size = libc::sysconf(libc::_SC_PAGESIZE).try_into().unwrap();
            }
        }

        if align > self.page_size || size > self.page_size {
            unsafe { self.alloc_multi_page(layout, zeroed) }
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
                            if zeroed {
                                // No need to zero out more than was specifically requested
                                ret.write_bytes(0, layout.size());
                            }
                            return ret;
                        }
                    }
                }
            }

            // We get here only if there's no space in our existing pages
            self.add_page();
            unsafe { self.alloc_inner(layout, zeroed) }
        }
    }

    /// SAFETY: Same as `alloc_inner()` with the added requirement that `layout`
    /// must ask for a size larger than the host pagesize.
    unsafe fn alloc_multi_page(&mut self, layout: Layout, zeroed: bool) -> *mut u8 {
        let ret =
            unsafe { if zeroed { alloc::alloc_zeroed(layout) } else { alloc::alloc(layout) } };
        self.huge_allocs.push((ret, layout.size()));
        ret
    }

    /// Deallocates a pointer from the isolated allocator.
    ///
    /// SAFETY: This pointer must have been allocated with `MachineAlloc::alloc()`
    /// (or `alloc_zeroed()`) with the same layout as the one passed.
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
            // And also zero out the page contents!
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
        let idx = self
            .huge_allocs
            .iter()
            .position(|pg| ptr.addr() == pg.0.addr())
            .expect("Freeing unallocated pages");
        let ptr = self.huge_allocs.remove(idx).0;
        unsafe {
            alloc::dealloc(ptr, layout);
        }
    }
}
