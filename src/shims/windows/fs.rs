use std::fs::{File, Metadata, OpenOptions};
use std::io;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::shims::files::FileDescription;
use crate::shims::windows::handle::{EvalContextExt as _, Handle};
use crate::*;

#[derive(Debug)]
pub struct FileHandle {
    pub(crate) file: File,
    pub(crate) writable: bool,
}

impl FileDescription for FileHandle {
    fn name(&self) -> &'static str {
        "file"
    }

    fn close<'tcx>(
        self: Box<Self>,
        communicate_allowed: bool,
        _ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx, io::Result<()>> {
        assert!(communicate_allowed, "isolation should have prevented even opening a file");
        // We sync the file if it was opened in a mode different than read-only.
        if self.writable {
            // `File::sync_all` does the checks that are done when closing a file. We do this to
            // to handle possible errors correctly.
            let result = self.file.sync_all();
            // Now we actually close the file and return the result.
            drop(*self);
            interp_ok(result)
        } else {
            // We drop the file, this closes it but ignores any errors
            // produced when closing it. This is done because
            // `File::sync_all` cannot be done over files like
            // `/dev/urandom` which are read-only. Check
            // https://github.com/rust-lang/miri/issues/999#issuecomment-568920439
            // for a deeper discussion.
            drop(*self);
            interp_ok(Ok(()))
        }
    }

    fn metadata<'tcx>(&self) -> InterpResult<'tcx, io::Result<Metadata>> {
        interp_ok(self.file.metadata())
    }

    fn is_tty(&self, communicate_allowed: bool) -> bool {
        communicate_allowed && self.file.is_terminal()
    }
}

#[derive(Debug)]
pub struct DirHandle {
    pub(crate) path: PathBuf,
}

impl FileDescription for DirHandle {
    fn name(&self) -> &'static str {
        "directory"
    }

    fn metadata<'tcx>(&self) -> InterpResult<'tcx, io::Result<Metadata>> {
        interp_ok(self.path.metadata())
    }

    fn close<'tcx>(
        self: Box<Self>,
        _communicate_allowed: bool,
        _ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx, io::Result<()>> {
        interp_ok(Ok(()))
    }
}

#[derive(Debug)]
pub struct MetadataHandle {
    pub(crate) meta: Metadata,
}

impl FileDescription for MetadataHandle {
    fn name(&self) -> &'static str {
        "metadata-only"
    }

    fn metadata<'tcx>(&self) -> InterpResult<'tcx, io::Result<Metadata>> {
        interp_ok(Ok(self.meta.clone()))
    }

    fn close<'tcx>(
        self: Box<Self>,
        _communicate_allowed: bool,
        _ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx, io::Result<()>> {
        interp_ok(Ok(()))
    }
}

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
#[allow(non_snake_case)]
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    fn CreateFileW(
        &mut self,
        file_name: &OpTy<'tcx>,            // LPCWSTR
        desired_access: &OpTy<'tcx>,       // DWORD
        share_mode: &OpTy<'tcx>,           // DWORD
        security_attributes: &OpTy<'tcx>,  // LPSECURITY_ATTRIBUTES
        creation_disposition: &OpTy<'tcx>, // DWORD
        flags_and_attributes: &OpTy<'tcx>, // DWORD
        template_file: &OpTy<'tcx>,        // HANDLE
    ) -> InterpResult<'tcx, Handle> {
        // ^ Returns HANDLE
        let this = self.eval_context_mut();
        this.assert_target_os("windows", "CreateFileW");
        this.check_no_isolation("`CreateFileW`")?;

        let file_name = this.read_path_from_wide_str(this.read_pointer(file_name)?)?;
        let mut desired_access = this.read_scalar(desired_access)?.to_u32()?;
        let share_mode = this.read_scalar(share_mode)?.to_u32()?;
        let security_attributes = this.read_pointer(security_attributes)?;
        let creation_disposition = this.read_scalar(creation_disposition)?.to_u32()?;
        let flags_and_attributes = this.read_scalar(flags_and_attributes)?.to_u32()?;
        let template_file = this.read_target_usize(template_file)?;

        let generic_read = this.eval_windows_u32("c", "GENERIC_READ");
        let generic_write = this.eval_windows_u32("c", "GENERIC_WRITE");

        let file_share_delete = this.eval_windows_u32("c", "FILE_SHARE_DELETE");
        let file_share_read = this.eval_windows_u32("c", "FILE_SHARE_READ");
        let file_share_write = this.eval_windows_u32("c", "FILE_SHARE_WRITE");

        let create_always = this.eval_windows_u32("c", "CREATE_ALWAYS");
        let create_new = this.eval_windows_u32("c", "CREATE_NEW");
        let open_always = this.eval_windows_u32("c", "OPEN_ALWAYS");
        let open_existing = this.eval_windows_u32("c", "OPEN_EXISTING");
        let truncate_existing = this.eval_windows_u32("c", "TRUNCATE_EXISTING");

        let file_attribute_normal = this.eval_windows_u32("c", "FILE_ATTRIBUTE_NORMAL");
        // This must be passed to allow getting directory handles. If not passed, we error on trying
        // to open directories below
        let file_flag_backup_semantics = this.eval_windows_u32("c", "FILE_FLAG_BACKUP_SEMANTICS");
        let file_flag_open_reparse_point =
            this.eval_windows_u32("c", "FILE_FLAG_OPEN_REPARSE_POINT");

        if share_mode != (file_share_delete | file_share_read | file_share_write) {
            throw_unsup_format!("CreateFileW: Unsupported share mode: {share_mode}");
        }
        if !this.ptr_is_null(security_attributes)? {
            throw_unsup_format!("CreateFileW: Security attributes are not supported");
        }

        let flags_and_attributes = match flags_and_attributes {
            0 => file_attribute_normal,
            _ => flags_and_attributes,
        };
        if !(file_attribute_normal | file_flag_backup_semantics | file_flag_open_reparse_point)
            & flags_and_attributes
            != 0
        {
            throw_unsup_format!(
                "CreateFileW: Unsupported flags_and_attributes: {flags_and_attributes}"
            );
        }

        if flags_and_attributes & file_flag_open_reparse_point != 0
            && creation_disposition == create_always
        {
            throw_machine_stop!(TerminationInfo::Abort("Invalid CreateFileW argument combination: FILE_FLAG_OPEN_REPARSE_POINT with CREATE_ALWAYS".to_string()));
        }

        if template_file != 0 {
            throw_unsup_format!("CreateFileW: Template files are not supported");
        }

        let is_dir = file_name.is_dir();

        if flags_and_attributes == file_attribute_normal && is_dir {
            this.set_last_error(IoError::WindowsError("ERROR_ACCESS_DENIED"))?;
            return interp_ok(Handle::Invalid);
        }

        let desired_read = desired_access & generic_read != 0;
        let desired_write = desired_access & generic_write != 0;

        let mut options = OpenOptions::new();
        if desired_read {
            desired_access &= !generic_read;
            options.read(true);
        }
        if desired_write {
            desired_access &= !generic_write;
            options.write(true);
        }

        if desired_access != 0 {
            throw_unsup_format!(
                "CreateFileW: Unsupported bits set for access mode: {desired_access:#x}"
            );
        }

        // This is racy, but there doesn't appear to be an std API that both succeeds if a file
        // exists but tells us it isn't new. Either we accept racing one way or another, or we
        // use an iffy heuristic like file creation time. This implementation prefers to fail in the
        // direction of erroring more often.
        let exists = file_name.exists();

        if creation_disposition == create_always {
            // Per the documentation:
            // If the specified file exists and is writable, the function truncates the file, the
            // function succeeds, and last-error code is set to ERROR_ALREADY_EXISTS.
            // If the specified file does not exist and is a valid path, a new file is created, the
            // function succeeds, and the last-error code is set to zero.
            // https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew
            if exists {
                this.set_last_error(IoError::WindowsError("ERROR_ALREADY_EXISTS"))?;
            } else {
                this.set_last_error(IoError::Raw(Scalar::from_u32(0)))?;
            }
            options.create(true);
            options.truncate(true);
        } else if creation_disposition == create_new {
            options.create_new(true);
            // Per `create_new` documentation:
            // If the specified file does not exist and is a valid path to a writable location, the
            // function creates a file and the last-error code is set to zero.
            // https://doc.rust-lang.org/std/fs/struct.OpenOptions.html#method.create_new
            if !desired_write {
                options.append(true);
            }
        } else if creation_disposition == open_always {
            // Per the documentation:
            // If the specified file exists, the function succeeds and the last-error code is set
            // to ERROR_ALREADY_EXISTS.
            // If the specified file does not exist and is a valid path to a writable location, the
            // function creates a file and the last-error code is set to zero.
            // https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew
            if exists {
                this.set_last_error(IoError::WindowsError("ERROR_ALREADY_EXISTS"))?;
            } else {
                this.set_last_error(IoError::Raw(Scalar::from_u32(0)))?;
            }
            options.create(true);
        } else if creation_disposition == open_existing {
            // Nothing
        } else if creation_disposition == truncate_existing {
            options.truncate(true);
        } else {
            throw_unsup_format!(
                "CreateFileW: Unsupported creation disposition: {creation_disposition}"
            );
        }

        let handle = if is_dir {
            let fh = &mut this.machine.fds;
            let fd_num = fh.insert_new(DirHandle { path: file_name });
            Ok(Handle::File(fd_num))
        } else if creation_disposition == open_existing && !(desired_read || desired_write) {
            // Windows supports handles with no permissions. These allow things such as reading
            // metadata, but not file content.
            let fh = &mut this.machine.fds;
            file_name.metadata().map(|meta| {
                let fd_num = fh.insert_new(MetadataHandle { meta });
                Handle::File(fd_num)
            })
        } else {
            options.open(file_name).map(|file| {
                let fh = &mut this.machine.fds;
                let fd_num = fh.insert_new(FileHandle { file, writable: desired_write });
                Handle::File(fd_num)
            })
        };

        match handle {
            Ok(handle) => interp_ok(handle),
            Err(e) => {
                this.set_last_error(e)?;
                interp_ok(Handle::Invalid)
            }
        }
    }

    fn GetFileInformationByHandle(
        &mut self,
        file: &OpTy<'tcx>,             // HANDLE
        file_information: &OpTy<'tcx>, // LPBY_HANDLE_FILE_INFORMATION
    ) -> InterpResult<'tcx, Scalar> {
        // ^ Returns BOOL (i32 on Windows)
        let this = self.eval_context_mut();
        this.assert_target_os("windows", "GetFileInformationByHandle");
        this.check_no_isolation("`GetFileInformationByHandle`")?;

        let file = this.read_handle(file, "GetFileInformationByHandle")?;
        let file_information = this.deref_pointer_as(
            file_information,
            this.windows_ty_layout("BY_HANDLE_FILE_INFORMATION"),
        )?;

        let fd_num = if let Handle::File(fd_num) = file {
            fd_num
        } else {
            this.invalid_handle("GetFileInformationByHandle")?
        };

        let Some(desc) = this.machine.fds.get(fd_num) else {
            this.invalid_handle("GetFileInformationByHandle")?
        };

        let metadata = match desc.metadata()? {
            Ok(meta) => meta,
            Err(e) => {
                this.set_last_error(e)?;
                return interp_ok(this.eval_windows("c", "FALSE"));
            }
        };

        let size = metadata.len();

        let file_type = metadata.file_type();
        let attributes = if file_type.is_dir() {
            this.eval_windows_u32("c", "FILE_ATTRIBUTE_DIRECTORY")
        } else if file_type.is_file() {
            this.eval_windows_u32("c", "FILE_ATTRIBUTE_NORMAL")
        } else {
            this.eval_windows_u32("c", "FILE_ATTRIBUTE_DEVICE")
        };

        let created = extract_windows_epoch(this, metadata.created())?.unwrap_or((0, 0));
        let accessed = extract_windows_epoch(this, metadata.accessed())?.unwrap_or((0, 0));
        let written = extract_windows_epoch(this, metadata.modified())?.unwrap_or((0, 0));

        this.write_int_fields_named(&[("dwFileAttributes", attributes.into())], &file_information)?;
        write_filetime_field(this, &file_information, "ftCreationTime", created)?;
        write_filetime_field(this, &file_information, "ftLastAccessTime", accessed)?;
        write_filetime_field(this, &file_information, "ftLastWriteTime", written)?;
        this.write_int_fields_named(
            &[
                ("dwVolumeSerialNumber", 0),
                ("nFileSizeHigh", (size >> 32).into()),
                ("nFileSizeLow", (size & 0xFFFFFFFF).into()),
                ("nNumberOfLinks", 1),
                ("nFileIndexHigh", 0),
                ("nFileIndexLow", 0),
            ],
            &file_information,
        )?;

        interp_ok(this.eval_windows("c", "TRUE"))
    }
}

/// Windows FILETIME is measured in 100-nanosecs since 1601
fn extract_windows_epoch<'tcx>(
    ecx: &MiriInterpCx<'tcx>,
    time: io::Result<SystemTime>,
) -> InterpResult<'tcx, Option<(u32, u32)>> {
    match time.ok() {
        Some(time) => {
            let duration = ecx.system_time_since_windows_epoch(&time)?;
            let duration_ticks = ecx.windows_ticks_for(duration)?;
            #[allow(clippy::cast_possible_truncation)]
            interp_ok(Some((duration_ticks as u32, (duration_ticks >> 32) as u32)))
        }
        None => interp_ok(None),
    }
}

fn write_filetime_field<'tcx>(
    cx: &mut MiriInterpCx<'tcx>,
    val: &MPlaceTy<'tcx>,
    name: &str,
    (low, high): (u32, u32),
) -> InterpResult<'tcx> {
    cx.write_int_fields_named(
        &[("dwLowDateTime", low.into()), ("dwHighDateTime", high.into())],
        &cx.project_field_named(val, name)?,
    )
}
