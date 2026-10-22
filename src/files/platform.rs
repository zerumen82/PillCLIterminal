use std::{
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions},
    io,
    mem::{offset_of, size_of, zeroed},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Path, PathBuf},
};

use windows_sys::{
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES,
        Storage::FileSystem::{
            FILE_CREATE, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_IF,
            FILE_OPEN_REPARSE_POINT, FILE_OVERWRITE_IF, FILE_SYNCHRONOUS_IO_NONALERT, NtCreateFile,
            NtSetInformationFile,
        },
    },
    Win32::{
        Foundation::{
            ERROR_NO_MORE_FILES, GENERIC_READ, GENERIC_WRITE, LocalFree, RtlNtStatusToDosError,
            UNICODE_STRING,
        },
        Security::{
            Authorization::{GetSecurityInfo, SE_FILE_OBJECT, SetSecurityInfo},
            DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, GetSecurityDescriptorControl,
            OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SE_DACL_PROTECTED,
            UNPROTECTED_DACL_SECURITY_INFORMATION,
        },
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ATTRIBUTE_DIRECTORY,
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_ID_BOTH_DIR_INFO, FILE_RENAME_INFO, FILE_SHARE_READ, FILE_SHARE_WRITE,
            FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo, GetFileInformationByHandle,
            GetFileInformationByHandleEx, GetFinalPathNameByHandleW, SYNCHRONIZE, WRITE_DAC,
            WRITE_OWNER,
        },
        System::IO::IO_STATUS_BLOCK,
    },
};

use super::FileError;

pub(super) fn pin_chain(path: &Path) -> Result<Vec<File>, FileError> {
    let mut ancestors: Vec<_> = path.ancestors().collect();
    ancestors.reverse();
    let mut pins = Vec::new();
    for ancestor in ancestors {
        let pin = if let Some(parent) = pins.last() {
            open_directory_at(
                parent,
                ancestor
                    .file_name()
                    .ok_or_else(|| FileError::Invalid("ancestro sin nombre".into()))?,
                false,
            )?
        } else {
            pin_volume(ancestor)?
        };
        pins.push(pin);
    }
    Ok(pins)
}

fn pin_volume(path: &Path) -> Result<File, FileError> {
    let file = OpenOptions::new()
        .access_mode(GENERIC_READ)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    check_directory(&file)?;
    Ok(file)
}

pub(super) fn open_file_at(parent: &File, name: &OsStr, edit: bool) -> Result<File, FileError> {
    let file = open_at(
        parent,
        name,
        GENERIC_READ | if edit { DELETE } else { 0 },
        FILE_SHARE_READ,
        FILE_OPEN,
        FILE_NON_DIRECTORY_FILE,
    )?;
    check_regular(&file)?;
    Ok(file)
}

pub(super) fn create_file_at(parent: &File, name: &OsStr) -> Result<File, FileError> {
    let file = open_at(
        parent,
        name,
        GENERIC_READ | GENERIC_WRITE | DELETE | WRITE_DAC | WRITE_OWNER,
        FILE_SHARE_READ,
        FILE_CREATE,
        FILE_NON_DIRECTORY_FILE,
    )?;
    check_regular(&file)?;
    Ok(file)
}

/// Sustituye el contenido si el nombre ya existe. Rechaza reparse y hardlinks.
pub(super) fn overwrite_file_at(parent: &File, name: &OsStr) -> Result<File, FileError> {
    let file = open_at(
        parent,
        name,
        GENERIC_READ | GENERIC_WRITE,
        FILE_SHARE_READ,
        FILE_OVERWRITE_IF,
        FILE_NON_DIRECTORY_FILE,
    )?;
    check_regular(&file)?;
    Ok(file)
}

pub(super) fn open_directory_at(
    parent: &File,
    name: &OsStr,
    create: bool,
) -> Result<File, FileError> {
    let file = open_at(
        parent,
        name,
        GENERIC_READ,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        if create { FILE_OPEN_IF } else { FILE_OPEN },
        FILE_DIRECTORY_FILE,
    )?;
    check_directory(&file)?;
    Ok(file)
}

pub(super) fn create_directory_at(parent: &File, name: &OsStr) -> Result<File, FileError> {
    let file = open_at(
        parent,
        name,
        GENERIC_READ,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        FILE_CREATE,
        FILE_DIRECTORY_FILE,
    )?;
    check_directory(&file)?;
    Ok(file)
}

pub(super) fn open_any_at(parent: &File, name: &OsStr) -> Result<File, FileError> {
    let file = open_at(
        parent,
        name,
        GENERIC_READ,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        FILE_OPEN,
        0,
    )?;
    if information(&file)?.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(FileError::Protected("reparse no admitido".into()));
    }
    Ok(file)
}

fn open_at(
    parent: &File,
    name: &OsStr,
    access: u32,
    share: u32,
    disposition: u32,
    options: u32,
) -> Result<File, FileError> {
    let mut name: Vec<u16> = name.encode_wide().collect();
    let length = u16::try_from(name.len() * 2)
        .map_err(|_| FileError::Invalid("nombre demasiado largo".into()))?;
    let mut unicode = UNICODE_STRING {
        Length: length,
        MaximumLength: length,
        Buffer: name.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &mut unicode,
        Attributes: 0x40,
        SecurityDescriptor: std::ptr::null_mut(),
        SecurityQualityOfService: std::ptr::null_mut(),
    };
    let mut handle = std::ptr::null_mut();
    // SAFETY: buffers y RootDirectory vivos hasta la llamada síncrona; ningún
    // componente intermedio se vuelve a resolver por una ruta absoluta.
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            access | SYNCHRONIZE,
            &attributes,
            &mut zeroed(),
            std::ptr::null(),
            0,
            share,
            disposition,
            options | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            std::ptr::null(),
            0,
        )
    };
    if status < 0 {
        return Err(nt_error(status));
    }
    // SAFETY: NtCreateFile ha entregado un handle nuevo y propio.
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn nt_error(status: i32) -> FileError {
    // SAFETY: conversión pura de NTSTATUS devuelto por Windows.
    io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32).into()
}

pub(super) fn check_directory(file: &File) -> Result<(), FileError> {
    let info = information(file)?;
    if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
        || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(FileError::Protected("directorio reparse o inválido".into()));
    }
    Ok(())
}

pub(super) fn is_directory(file: &File) -> Result<bool, FileError> {
    Ok(information(file)?.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0)
}

/// Enumeración por el handle fijado; no sigue un junction cambiado concurrentemente.
pub(super) fn entries(directory: &File, limit: usize) -> Result<(Vec<OsString>, bool), FileError> {
    check_directory(directory)?;
    // Un nuevo FILE_OBJECT tiene cursor propio. try_clone compartiría el cursor
    // y dos exploraciones podrían interrumpirse sus enumeraciones.
    let iterator = open_directory_at(directory, OsStr::new(""), false)?;
    let mut names = Vec::new();
    // Una entrada admite hasta 32767 caracteres UTF-16; buffer alineado.
    let mut buffer =
        vec![0u64; (size_of::<FILE_ID_BOTH_DIR_INFO>() + u16::MAX as usize).div_ceil(8)];
    let mut restart = true;
    loop {
        let class = if restart {
            FileIdBothDirectoryRestartInfo
        } else {
            FileIdBothDirectoryInfo
        };
        restart = false;
        // SAFETY: buffer alineado, tamaño suficiente, handle de directorio vivo.
        if unsafe {
            GetFileInformationByHandleEx(
                iterator.as_raw_handle(),
                class,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * 8) as u32,
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                break;
            }
            return Err(error.into());
        }
        let mut position = 0usize;
        loop {
            // SAFETY: Windows construye lista dentro del buffer; se comprueban
            // offsets y longitud antes de acceder al nombre variable.
            let base = unsafe { buffer.as_ptr().cast::<u8>().add(position) };
            let header = unsafe { &*base.cast::<FILE_ID_BOTH_DIR_INFO>() };
            let name_offset = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
            let name_bytes = header.FileNameLength as usize;
            if position + name_offset + name_bytes > buffer.len() * 8
                || !name_bytes.is_multiple_of(2)
            {
                return Err(FileError::Invalid("enumeración Windows inválida".into()));
            }
            let name = OsString::from_wide(unsafe {
                std::slice::from_raw_parts(base.add(name_offset).cast(), name_bytes / 2)
            });
            if name != "." && name != ".." {
                if names.len() == limit {
                    return Ok((names, true));
                }
                names.push(name);
            }
            if header.NextEntryOffset == 0 {
                break;
            }
            position += header.NextEntryOffset as usize;
            if position + size_of::<FILE_ID_BOTH_DIR_INFO>() > buffer.len() * 8 {
                return Err(FileError::Invalid("offset de enumeración inválido".into()));
            }
        }
    }
    Ok((names, false))
}

/// Copia por handles antes de cambiar nombres. Si no se pueden preservar estos
/// permisos, la edición falla con la base todavía en su destino original.
pub(super) fn copy_security(original: &File, staged: &File) -> Result<(), FileError> {
    let mut owner = std::ptr::null_mut();
    let mut group = std::ptr::null_mut();
    let mut dacl = std::ptr::null_mut();
    let mut descriptor = std::ptr::null_mut();
    let information =
        OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    // SAFETY: salidas vivas; GetSecurityInfo devuelve descriptor LocalAlloc.
    let error = unsafe {
        GetSecurityInfo(
            original.as_raw_handle(),
            SE_FILE_OBJECT,
            information,
            &mut owner,
            &mut group,
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32).into());
    }
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: owner/group/dacl apuntan al descriptor todavía vivo.
    let result = unsafe {
        if GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) == 0 {
            Err(io::Error::last_os_error())
        } else {
            let inheritance = if control & SE_DACL_PROTECTED != 0 {
                PROTECTED_DACL_SECURITY_INFORMATION
            } else {
                UNPROTECTED_DACL_SECURITY_INFORMATION
            };
            let error = SetSecurityInfo(
                staged.as_raw_handle(),
                SE_FILE_OBJECT,
                information | inheritance,
                owner,
                group,
                dacl,
                std::ptr::null(),
            );
            if error == 0 {
                Ok(())
            } else {
                Err(io::Error::from_raw_os_error(error as i32))
            }
        }
    };
    // SAFETY: memoria obtenida de GetSecurityInfo, ya no se usa.
    unsafe {
        LocalFree(descriptor);
    }
    result.map_err(FileError::Io)
}

fn information(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION, FileError> {
    // SAFETY: estructura inicializada y handle vivo poseído por File.
    let mut information = unsafe { zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(information)
}

pub(super) fn check_regular(file: &File) -> Result<(), FileError> {
    let metadata = information(file)?;
    if metadata.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY) != 0
        || metadata.nNumberOfLinks != 1
    {
        return Err(FileError::Protected(
            "reparse, directorio o hardlink no admitido".into(),
        ));
    }
    Ok(())
}

pub(super) fn final_path(file: &File) -> Result<PathBuf, FileError> {
    // SAFETY: primer sondeo sin buffer; segundo buffer con longitud comprobada.
    let needed =
        unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), std::ptr::null_mut(), 0, 0) };
    if needed == 0 {
        return Err(io::Error::last_os_error().into());
    }
    let mut buffer = vec![0u16; needed as usize + 1];
    let length = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            0,
        )
    };
    if length == 0 || length as usize >= buffer.len() {
        return Err(io::Error::last_os_error().into());
    }
    Ok(PathBuf::from(std::ffi::OsString::from_wide(
        &buffer[..length as usize],
    )))
}

/// Rename por el handle bloqueado. ReplaceIfExists siempre false.
pub(super) fn rename_at(file: &File, parent: &File, destination: &OsStr) -> Result<(), FileError> {
    let name: Vec<u16> = destination.encode_wide().collect();
    let offset = offset_of!(FILE_RENAME_INFO, FileName);
    // El terminador queda fuera de FileNameLength.
    let bytes = offset + (name.len() + 1) * size_of::<u16>();
    let length =
        u32::try_from(bytes).map_err(|_| FileError::Invalid("ruta demasiado larga".into()))?;
    // Vec<usize> proporciona la alineación de FILE_RENAME_INFO; no Vec<u8>.
    let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
    let pointer = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // SAFETY: storage alineado con al menos offset+bytes UTF-16. Ceros fijan
    // ReplaceIfExists=false y RootDirectory fijado; ambos handles vivos.
    unsafe {
        (*pointer).RootDirectory = parent.as_raw_handle();
        (*pointer).FileNameLength = (name.len() * size_of::<u16>()) as u32;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            storage.as_mut_ptr().cast::<u8>().add(offset).cast(),
            name.len(),
        );
        let mut status_block: IO_STATUS_BLOCK = zeroed();
        let status = NtSetInformationFile(
            file.as_raw_handle(),
            &mut status_block,
            pointer.cast(),
            length,
            10,
        );
        if status < 0 {
            return Err(nt_error(status));
        }
    }
    Ok(())
}
