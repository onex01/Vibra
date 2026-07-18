pub mod ramfs;

pub use ramfs::{
    init_filesystem, list_entries, create_file, create_dir,
    write_file, read_file, remove_entry, fs_count, FileType
};