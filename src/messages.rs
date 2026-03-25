use anyhow::Result;
use bytesize::ByteSize;

pub enum BackendCommand {
    Refresh,
    Write {
        space_to_leave: ByteSize,
        selected_index: usize,
        keep_local: bool,
    },
    Exit,
}

pub enum BackendWrite {
    InProgress(u64, u64, &'static str),
    Completed(Result<()>),
}

pub enum BackendEvent {
    RefreshFinished(anyhow::Result<Vec<slint::SharedString>>),
    Write(BackendWrite),
}
