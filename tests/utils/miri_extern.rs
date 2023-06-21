#![allow(dead_code)]

#[repr(C)]
/// Layout of the return value of `miri_resolve_frame`,
/// with fields in the exact same order.
pub struct MiriFrame {
    // The size of the name of the function being executed, encoded in UTF-8
    pub name_len: usize,
    // The size of filename of the function being executed, encoded in UTF-8
    pub filename_len: usize,
    // The line number currently being executed in `filename`, starting from '1'.
    pub lineno: u32,
    // The column number currently being executed in `filename`, starting from '1'.
    pub colno: u32,
    // The function pointer to the function currently being executed.
    // This can be compared against function pointers obtained by
    // casting a function (e.g. `my_fn as *mut ()`)
    pub fn_ptr: *mut (),
}

#[cfg(miri)]
extern "Rust" {
    /// Miri-provided extern function to mark the block `ptr` points to as a "root"
    /// for some static memory. This memory and everything reachable by it is not
    /// considered leaking even if it still exists when the program terminates.
    ///
    /// `ptr` has to point to the beginning of an allocated block.
    pub fn miri_static_root(ptr: *const u8);

    // Miri-provided extern function to get the amount of frames in the current backtrace.
    // The `flags` argument must be `0`.
    pub fn miri_backtrace_size(flags: u64) -> usize;

    /// Miri-provided extern function to obtain a backtrace of the current call stack.
    /// This writes a slice of pointers into `buf` - each pointer is an opaque value
    /// that is only useful when passed to `miri_resolve_frame`.
    /// `buf` must have `miri_backtrace_size(0) * pointer_size` bytes of space.
    /// The `flags` argument must be `1`.
    pub fn miri_get_backtrace(flags: u64, buf: *mut *mut ());

    /// Miri-provided extern function to resolve a frame pointer obtained
    /// from `miri_get_backtrace`. The `flags` argument must be `1`.
    ///
    /// This function can be called on any thread (not just the one which obtained `frame`).
    pub fn miri_resolve_frame(frame: *mut (), flags: u64) -> MiriFrame;

    /// Miri-provided extern function to get the name and filename of the frame provided by `miri_resolve_frame`.
    /// `name_buf` and `filename_buf` should be allocated with the `name_len` and `filename_len` fields of `MiriFrame`.
    /// The flags argument must be `0`.
    pub fn miri_resolve_frame_names(
        ptr: *mut (),
        flags: u64,
        name_buf: *mut u8,
        filename_buf: *mut u8,
    );

    /// Miri-provided extern function to begin unwinding with the given payload.
    ///
    /// This is internal and unstable and should not be used; we give it here
    /// just to be complete.
    pub fn miri_start_panic(payload: *mut u8) -> !;

    /// Miri-provided extern function to get the internal unique identifier for the allocation that a pointer
    /// points to. If this pointer is invalid (not pointing to an allocation), interpretation will abort.
    ///
    /// This is only useful as an input to `miri_print_borrow_stacks`, and it is a separate call because
    /// getting a pointer to an allocation at runtime can change the borrow stacks in the allocation.
    /// This function should be considered unstable. It exists only to support `miri_print_borrow_state` and so
    /// inherits all of its instability.
    pub fn miri_get_alloc_id(ptr: *const ()) -> u64;

    /// Miri-provided extern function to print (from the interpreter, not the program) the contents of all
    /// borrows in an allocation.
    ///
    /// If Stacked Borrows is running, this prints all the stacks. The leftmost tag is the bottom of the stack.
    ///
    /// If Tree borrows is running, this prints on the left the permissions of each tag on each range,
    /// an on the right the tree structure of the tags. If some tags were named via `miri_pointer_name`,
    /// their names appear here.
    ///
    /// If additionally `show_unnamed` is `false` then tags that did *not* receive a name will be hidden.
    /// Ensure that either the important tags have been named, or `show_unnamed = true`.
    /// Note: as Stacked Borrows does not have tag names at all, `show_unnamed` is ignored and all tags are shown.
    /// In general, unless you strongly want some tags to be hidden (as is the case in `tree-borrows` tests),
    /// `show_unnamed = true` should be the default.
    ///
    /// The format of what this emits is unstable and may change at any time. In particular, users should be
    /// aware that Miri will periodically attempt to garbage collect the contents of all stacks. Callers of
    /// this function may wish to pass `-Zmiri-tag-gc=0` to disable the GC.
    ///
    /// This function is extremely unstable. At any time the format of its output may change, its signature may
    /// change, or it may be removed entirely.
    pub fn miri_print_borrow_state(alloc_id: u64, show_unnamed: bool);

    /// Miri-provided extern function to associate a name to a tag.
    /// Typically the name given would be the name of the program variable that holds the pointer.
    /// Unreachable tags can still be accessed through a combination of `miri_tree_nth_parent` and
    /// `miri_tree_common_ancestor`.
    ///
    /// This function does nothing under Stacked Borrows, since Stacked Borrows's implementation
    /// of `miri_print_borrow_state` does not show the names.
    ///
    /// Under Tree Borrows, the names also appear in error messages.
    pub fn miri_pointer_name(ptr: *const (), name: &[u8]);

    /// Miri-provided extern function to forge the provenance of a pointer.
    /// Use only for debugging and diagnostics.
    ///
    /// Under Tree Borrows, this can be used in conjunction with `miri_pointer_name`
    /// to access a tag that is not directly accessible in the program: the pointer
    /// returned has the same address as `ptr` and is in the same allocation, but has
    /// the `tag` of the `nb`'th parent above `ptr`.
    ///
    /// `ptr` must not be a wildcard pointer.
    /// The output can change based on implementation details of `rustc` as well as
    /// `miri` flags such as `-Zmiri-unique-is-unique`.
    /// Behavior might be even more unpredictable without `-Zmiri-tag-gc=0`.
    ///
    /// Prefer the more stable `miri_tree_common_ancestor` when possible.
    /// In particular when `nb > 1` it is a sign that this function might not be
    /// the right one to use.
    ///
    /// Example usage: naming the caller-retagged parent of a function argument
    /// ```rs
    /// foo(&*x);
    /// fn foo(x: &T) {
    ///     // `x` was reborrowed once by the caller and once by the callee
    ///     // we can give a name to the callee's `x` with
    ///     miri_pointer_name(x as *const T as *const (), "callee:x".as_bytes());
    ///     // However the caller's `x` is not directly reachable.
    ///     miri_pointer_name(
    ///         miri_tree_nth_parent(x as *const T as *const (), 1),
    ///         "caller:x".as_bytes(),
    ///     );
    /// }
    /// ```
    ///
    /// This is a noop under Stacked Borrows.
    pub fn miri_tree_nth_parent(ptr: *const (), nb: u8) -> *const ();

    /// Miri-provided extern function to forge the provenance of a pointer.
    /// Use only for debugging and diagnostics.
    ///
    /// Under Tree Borrows, this can be used in conjunction with `miri_pointer_name`
    /// to access a tag that is not directly accessible in the program: the pointer
    /// returned has the same address as `ptr1` and is in the same allocation, but
    /// has the `tag` of the nearest common ancestor of `ptr1` and `ptr2`.
    ///
    /// Both `ptr1` and `ptr2` must not be wildcard pointers, and
    /// `ptr2` must be part of the same allocation as `ptr1`.
    ///
    /// Example usage:
    /// ```rs
    /// // If you have a function that returns a pointer and you use it as such:
    /// miri_pointer_name(
    ///     something() as *const (),
    ///     "something()".as_bytes(),
    /// );
    /// // you might not be naming the pointer you think you are: two invocations
    /// // of `something()` might yield different tags, and the one you gave a name to
    /// // might be later invalidated when the root pointer is still usable.
    /// // This occurs in particular for some `as_ptr` functions.
    ///
    /// // To give a name to the actual underlying pointer, you can use
    /// miri_pointer_name(
    ///     miri_tree_common_ancestor(
    ///         something() as *const (),
    ///         something() as *const (),
    ///     ),
    ///     "root of something()".as_bytes(),
    /// );
    /// ```
    ///
    /// This is a noop under Stacked Borrows.
    pub fn miri_tree_common_ancestor(ptr1: *const (), ptr2: *const ()) -> *const ();

    /// Miri-provided extern function to print (from the interpreter, not the
    /// program) the contents of a section of program memory, as bytes. Bytes
    /// written using this function will emerge from the interpreter's stdout.
    pub fn miri_write_to_stdout(bytes: &[u8]);

    /// Miri-provided extern function to print (from the interpreter, not the
    /// program) the contents of a section of program memory, as bytes. Bytes
    /// written using this function will emerge from the interpreter's stderr.
    pub fn miri_write_to_stderr(bytes: &[u8]);

    /// Miri-provided extern function to allocate memory from the interpreter.
    ///
    /// This is useful when no fundamental way of allocating memory is
    /// available, e.g. when using `no_std` + `alloc`.
    pub fn miri_alloc(size: usize, align: usize) -> *mut u8;

    /// Miri-provided extern function to deallocate memory.
    pub fn miri_dealloc(ptr: *mut u8, size: usize, align: usize);

    /// Convert a path from the host Miri runs on to the target Miri interprets.
    /// Performs conversion of path separators as needed.
    ///
    /// Usually Miri performs this kind of conversion automatically. However, manual conversion
    /// might be necessary when reading an environment variable that was set on the host
    /// (such as TMPDIR) and using it as a target path.
    ///
    /// Only works with isolation disabled.
    ///
    /// `in` must point to a null-terminated string, and will be read as the input host path.
    /// `out` must point to at least `out_size` many bytes, and the result will be stored there
    /// with a null terminator.
    /// Returns 0 if the `out` buffer was large enough, and the required size otherwise.
    pub fn miri_host_to_target_path(
        path: *const std::ffi::c_char,
        out: *mut std::ffi::c_char,
        out_size: usize,
    ) -> usize;
}
