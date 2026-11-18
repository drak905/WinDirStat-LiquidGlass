use super::{
    summarize_nodes, FrameKind, IpcBridgeError, IpcStreamFrame, ScanIngestor, ScanNode,
    SnapshotSummary, DEFAULT_FRAME_BATCH_SIZE, MSGPACK_CHUNK_MAGIC, TARGET_FRAME_INTERVAL_MS,
};
use serde::Deserialize;
use std::io::{BufRead, BufReader, Cursor, Read};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireFormat {
    Auto,
    Ndjson,
    MessagePack,
}

#[derive(Debug, Clone)]
pub struct StreamConfig {
    pub format: WireFormat,
    pub batch_size: usize,
    pub frame_interval: Duration,
    pub max_frame_bytes: usize,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            format: WireFormat::Auto,
            batch_size: DEFAULT_FRAME_BATCH_SIZE,
            frame_interval: Duration::from_millis(TARGET_FRAME_INTERVAL_MS),
            max_frame_bytes: 16 * 1024 * 1024,
        }
    }
}

pub fn encode_ndjson_frame(frame: &IpcStreamFrame) -> Result<String, IpcBridgeError> {
    let mut line = serde_json::to_string(frame)?;
    line.push('\n');
    Ok(line)
}

pub fn encode_msgpack_chunk(frame: &IpcStreamFrame) -> Result<Vec<u8>, IpcBridgeError> {
    let payload = rmp_serde::to_vec_named(frame)?;
    let len = u32::try_from(payload.len())
        .map_err(|_| IpcBridgeError::Protocol("frame exceeds u32 length".into()))?;
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&MSGPACK_CHUNK_MAGIC);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

#[derive(Deserialize)]
#[serde(untagged)]
enum NdjsonRecord {
    Frame(IpcStreamFrame),
    Node(ScanNode),
}

struct FrameBatcher {
    pending: Vec<ScanNode>,
    all_nodes: Vec<ScanNode>,
    seq: u64,
    scan_root: String,
    scanned_nodes: u64,
    total_bytes: u64,
    saw_final: bool,
    last_flush: Instant,
    batch_size: usize,
    frame_interval: Duration,
}

impl FrameBatcher {
    fn new(config: &StreamConfig) -> Self {
        let batch_size = config.batch_size.max(1);
        Self {
            pending: Vec::with_capacity(batch_size),
            all_nodes: Vec::new(),
            seq: 0,
            scan_root: String::new(),
            scanned_nodes: 0,
            total_bytes: 0,
            saw_final: false,
            last_flush: Instant::now(),
            batch_size,
            frame_interval: config.frame_interval,
        }
    }

    fn push_node(
        &mut self,
        node: ScanNode,
        ingestor: &dyn ScanIngestor,
    ) -> Result<(), IpcBridgeError> {
        if self.scan_root.is_empty() && node.parent_id.is_none() {
            self.scan_root = node.full_path.clone();
        }
        self.scanned_nodes += 1;
        if !node.is_dir {
            self.total_bytes = self.total_bytes.saturating_add(node.logical_size);
        }
        self.all_nodes.push(node.clone());
        self.pending.push(node);

        let interval_due =
            !self.frame_interval.is_zero() && self.last_flush.elapsed() >= self.frame_interval;
        if self.pending.len() >= self.batch_size || interval_due {
            self.flush(ingestor)?;
        }
        Ok(())
    }

    fn push_frame(
        &mut self,
        frame: IpcStreamFrame,
        ingestor: &dyn ScanIngestor,
    ) -> Result<(), IpcBridgeError> {
        self.flush(ingestor)?;

        if self.scan_root.is_empty() && !frame.scan_root.is_empty() {
            self.scan_root = frame.scan_root.clone();
        }
        for node in &frame.nodes {
            if self.scan_root.is_empty() && node.parent_id.is_none() {
                self.scan_root = node.full_path.clone();
            }
            self.scanned_nodes += 1;
            if !node.is_dir {
                self.total_bytes = self.total_bytes.saturating_add(node.logical_size);
            }
            self.all_nodes.push(node.clone());
        }
        self.seq = self.seq.max(frame.seq);
        if frame.is_final || frame.frame_type != FrameKind::Nodes {
            self.saw_final = true;
        }
        self.last_flush = Instant::now();
        ingestor.on_frame(frame)
    }

    fn flush(&mut self, ingestor: &dyn ScanIngestor) -> Result<(), IpcBridgeError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        self.seq += 1;
        let frame = IpcStreamFrame {
            frame_type: FrameKind::Nodes,
            seq: self.seq,
            scan_root: self.scan_root.clone(),
            scanned_nodes: self.scanned_nodes,
            total_bytes: self.total_bytes,
            is_final: false,
            error: None,
            nodes: std::mem::take(&mut self.pending),
        };
        self.last_flush = Instant::now();
        ingestor.on_frame(frame)
    }

    fn finish(mut self, ingestor: &dyn ScanIngestor) -> Result<SnapshotSummary, IpcBridgeError> {
        self.flush(ingestor)?;
        if !self.saw_final && !self.all_nodes.is_empty() {
            self.seq += 1;
            let final_frame = IpcStreamFrame {
                frame_type: FrameKind::Complete,
                seq: self.seq,
                scan_root: self.scan_root.clone(),
                scanned_nodes: self.scanned_nodes,
                total_bytes: self.total_bytes,
                is_final: true,
                error: None,
                nodes: Vec::new(),
            };
            ingestor.on_frame(final_frame)?;
        }
        let summary = summarize_nodes(&self.scan_root, &self.all_nodes, &[]);
        ingestor.on_complete(summary.clone())?;
        Ok(summary)
    }
}

fn read_exact_or_eof<R: Read>(reader: &mut R, buf: &mut [u8]) -> Result<usize, IpcBridgeError> {
    let mut total = 0;
    while total < buf.len() {
        let n = reader.read(&mut buf[total..])?;
        if n == 0 {
            break;
        }
        total += n;
    }
    Ok(total)
}

fn read_ndjson_loop<R: Read>(
    reader: R,
    batcher: &mut FrameBatcher,
    ingestor: &dyn ScanIngestor,
) -> Result<(), IpcBridgeError> {
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();
    let mut first = true;

    loop {
        line.clear();
        let bytes = buf_reader.read_line(&mut line)?;
        if bytes == 0 {
            break;
        }
        let mut trimmed = line.trim();
        if first {
            first = false;
            trimmed = trimmed.trim_start_matches('\u{FEFF}');
        }
        if trimmed.is_empty() {
            continue;
        }

        match serde_json::from_str::<NdjsonRecord>(trimmed)? {
            NdjsonRecord::Frame(frame) => batcher.push_frame(frame, ingestor)?,
            NdjsonRecord::Node(node) => batcher.push_node(node, ingestor)?,
        }
    }
    Ok(())
}

fn read_msgpack_loop<R: Read>(
    mut reader: R,
    initial_header: Option<[u8; 4]>,
    max_frame_bytes: usize,
    batcher: &mut FrameBatcher,
    ingestor: &dyn ScanIngestor,
) -> Result<(), IpcBridgeError> {
    let mut first_hdr = initial_header;
    loop {
        let mut hdr = [0u8; 4];
        if let Some(prefetched) = first_hdr.take() {
            hdr = prefetched;
        } else {
            let n = read_exact_or_eof(&mut reader, &mut hdr)?;
            if n == 0 {
                break;
            }
            if n < 4 {
                return Err(IpcBridgeError::Protocol(
                    "truncated MessagePack chunk header".into(),
                ));
            }
        }

        let len_bytes = if hdr == MSGPACK_CHUNK_MAGIC {
            let mut len_buf = [0u8; 4];
            let n = read_exact_or_eof(&mut reader, &mut len_buf)?;
            if n < 4 {
                return Err(IpcBridgeError::Protocol(
                    "truncated MessagePack chunk length".into(),
                ));
            }
            len_buf
        } else {
            hdr
        };

        let payload_len = u32::from_le_bytes(len_bytes) as usize;
        if payload_len == 0 || payload_len > max_frame_bytes {
            return Err(IpcBridgeError::Protocol(format!(
                "invalid MessagePack frame length: {payload_len}"
            )));
        }

        let mut payload = vec![0u8; payload_len];
        let n = read_exact_or_eof(&mut reader, &mut payload)?;
        if n < payload_len {
            return Err(IpcBridgeError::Protocol(
                "unexpected EOF inside MessagePack frame payload".into(),
            ));
        }

        let frame: IpcStreamFrame = rmp_serde::from_slice(&payload)?;
        batcher.push_frame(frame, ingestor)?;
    }
    Ok(())
}

pub fn read_framed_stream<R: Read>(
    mut reader: R,
    config: &StreamConfig,
    ingestor: &dyn ScanIngestor,
) -> Result<SnapshotSummary, IpcBridgeError> {
    let mut batcher = FrameBatcher::new(config);

    match config.format {
        WireFormat::Ndjson => {
            read_ndjson_loop(reader, &mut batcher, ingestor)?;
        }
        WireFormat::MessagePack => {
            read_msgpack_loop(reader, None, config.max_frame_bytes, &mut batcher, ingestor)?;
        }
        WireFormat::Auto => {
            let mut probe = [0u8; 4];
            let n = read_exact_or_eof(&mut reader, &mut probe)?;
            if n == 0 {
                return batcher.finish(ingestor);
            }
            if n == 4 && probe == MSGPACK_CHUNK_MAGIC {
                read_msgpack_loop(
                    reader,
                    Some(probe),
                    config.max_frame_bytes,
                    &mut batcher,
                    ingestor,
                )?;
            } else {
                let chained = Cursor::new(probe[..n].to_vec()).chain(reader);
                read_ndjson_loop(chained, &mut batcher, ingestor)?;
            }
        }
    }

    batcher.finish(ingestor)
}

#[cfg(windows)]
mod win_pipe {
    use super::*;
    use std::sync::Arc;
    use std::thread::{self, JoinHandle};
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_HANDLE_EOF,
        ERROR_IO_PENDING, ERROR_OPERATION_ABORTED, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
        GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL,
        FILE_FLAG_OVERLAPPED, OPEN_EXISTING, PIPE_ACCESS_INBOUND,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, WaitNamedPipeW,
        PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    use windows_sys::Win32::System::Threading::{
        CreateEventW, ResetEvent, SetEvent, WaitForMultipleObjects, INFINITE,
    };
    use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

    struct OwnedHandle(HANDLE);

    unsafe impl Send for OwnedHandle {}
    unsafe impl Sync for OwnedHandle {}

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }

    fn to_wide_null(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn normalize_pipe_name(raw: &str) -> Result<String, IpcBridgeError> {
        let trimmed = raw.trim().trim_matches('"').replace('/', "\\");
        let trimmed = trimmed.trim_end_matches('\\');
        if trimmed.is_empty() {
            return Err(IpcBridgeError::Protocol("pipe name cannot be empty".into()));
        }
        if trimmed.starts_with(r"\\") {
            return Ok(trimmed.to_string());
        }
        if trimmed.contains(':') {
            return Err(IpcBridgeError::Protocol(format!(
                "invalid Windows named pipe path: {raw}"
            )));
        }
        Ok(format!(r"\\.\pipe\{}", trimmed.trim_start_matches('\\')))
    }

    fn create_pipe_instance(wide_name: &[u16]) -> Result<OwnedHandle, IpcBridgeError> {
        let handle = unsafe {
            CreateNamedPipeW(
                wide_name.as_ptr(),
                PIPE_ACCESS_INBOUND | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                65536,
                65536,
                0,
                std::ptr::null(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(OwnedHandle(handle))
    }

    fn create_manual_event() -> Result<OwnedHandle, IpcBridgeError> {
        let evt = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if evt.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(OwnedHandle(evt))
    }

    enum ConnectOutcome {
        Connected,
        Stopped,
    }

    fn wait_for_client(
        pipe: HANDLE,
        io_event: HANDLE,
        stop_event: HANDLE,
    ) -> Result<ConnectOutcome, IpcBridgeError> {
        unsafe { ResetEvent(io_event) };
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        overlapped.hEvent = io_event;

        let ok = unsafe { ConnectNamedPipe(pipe, &mut overlapped) };
        if ok != 0 {
            return Ok(ConnectOutcome::Connected);
        }

        let err = unsafe { GetLastError() };
        if err == ERROR_PIPE_CONNECTED {
            return Ok(ConnectOutcome::Connected);
        }
        if err != ERROR_IO_PENDING {
            return Err(std::io::Error::from_raw_os_error(err as i32).into());
        }

        let handles = [io_event, stop_event];
        let wait = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
        if wait == WAIT_OBJECT_0 + 1 {
            unsafe {
                CancelIoEx(pipe, &overlapped);
            }
            return Ok(ConnectOutcome::Stopped);
        }
        if wait != WAIT_OBJECT_0 {
            return Err(std::io::Error::last_os_error().into());
        }

        let mut transferred = 0u32;
        if unsafe { GetOverlappedResult(pipe, &overlapped, &mut transferred, 0) } == 0 {
            let ov_err = unsafe { GetLastError() };
            if ov_err == ERROR_OPERATION_ABORTED {
                return Ok(ConnectOutcome::Stopped);
            }
            return Err(std::io::Error::from_raw_os_error(ov_err as i32).into());
        }
        Ok(ConnectOutcome::Connected)
    }

    struct OverlappedPipeReader {
        pipe: HANDLE,
        io_event: HANDLE,
        stop_event: HANDLE,
    }

    impl Read for OverlappedPipeReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            unsafe { ResetEvent(self.io_event) };
            let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
            overlapped.hEvent = self.io_event;

            let to_read = buf.len().min(u32::MAX as usize) as u32;
            let mut bytes_read = 0u32;
            let ok = unsafe {
                ReadFile(
                    self.pipe,
                    buf.as_mut_ptr(),
                    to_read,
                    &mut bytes_read,
                    &mut overlapped,
                )
            };
            if ok != 0 {
                return Ok(bytes_read as usize);
            }

            let err = unsafe { GetLastError() };
            if err == ERROR_BROKEN_PIPE || err == ERROR_HANDLE_EOF {
                return Ok(0);
            }
            if err != ERROR_IO_PENDING {
                return Err(std::io::Error::from_raw_os_error(err as i32));
            }

            let handles = [self.io_event, self.stop_event];
            let wait = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
            if wait == WAIT_OBJECT_0 + 1 {
                unsafe {
                    CancelIoEx(self.pipe, &overlapped);
                }
                return Ok(0);
            }
            if wait != WAIT_OBJECT_0 {
                return Err(std::io::Error::last_os_error());
            }

            if unsafe { GetOverlappedResult(self.pipe, &overlapped, &mut bytes_read, 0) } == 0 {
                let ov_err = unsafe { GetLastError() };
                if ov_err == ERROR_BROKEN_PIPE
                    || ov_err == ERROR_HANDLE_EOF
                    || ov_err == ERROR_OPERATION_ABORTED
                {
                    return Ok(0);
                }
                return Err(std::io::Error::from_raw_os_error(ov_err as i32));
            }
            Ok(bytes_read as usize)
        }
    }

    pub struct NamedPipeServer {
        pipe_name: String,
        stop_event: Arc<OwnedHandle>,
        worker: Option<JoinHandle<Result<(), IpcBridgeError>>>,
    }

    impl NamedPipeServer {
        pub fn start<I: ScanIngestor + 'static>(
            pipe_name: &str,
            config: StreamConfig,
            ingestor: Arc<I>,
        ) -> Result<Self, IpcBridgeError> {
            let normalized = normalize_pipe_name(pipe_name)?;
            let wide_name = to_wide_null(&normalized);
            let pipe_handle = create_pipe_instance(&wide_name)?;
            let stop_event = Arc::new(create_manual_event()?);
            let stop_for_thread = Arc::clone(&stop_event);

            let worker = thread::Builder::new()
                .name("wds-ipc-pipe-server".into())
                .spawn(move || -> Result<(), IpcBridgeError> {
                    let pipe_handle = pipe_handle;
                    let stop_for_thread = stop_for_thread;
                    let io_event = create_manual_event()?;
                    loop {
                        match wait_for_client(pipe_handle.0, io_event.0, stop_for_thread.0)? {
                            ConnectOutcome::Stopped => break,
                            ConnectOutcome::Connected => {
                                let reader = OverlappedPipeReader {
                                    pipe: pipe_handle.0,
                                    io_event: io_event.0,
                                    stop_event: stop_for_thread.0,
                                };
                                let read_res = read_framed_stream(reader, &config, ingestor.as_ref());
                                unsafe {
                                    DisconnectNamedPipe(pipe_handle.0);
                                }
                                read_res?;
                            }
                        }
                    }
                    Ok(())
                })?;

            Ok(Self {
                pipe_name: normalized,
                stop_event,
                worker: Some(worker),
            })
        }

        pub fn pipe_name(&self) -> &str {
            &self.pipe_name
        }

        pub fn stop(mut self) -> Result<(), IpcBridgeError> {
            unsafe {
                SetEvent(self.stop_event.0);
            }
            if let Some(worker) = self.worker.take() {
                worker.join().map_err(|_| {
                    IpcBridgeError::Protocol("named pipe worker thread panicked".into())
                })??;
            }
            Ok(())
        }
    }

    impl Drop for NamedPipeServer {
        fn drop(&mut self) {
            unsafe {
                SetEvent(self.stop_event.0);
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    pub fn write_to_named_pipe(
        pipe_name: &str,
        payload: &[u8],
        timeout_ms: u32,
    ) -> Result<(), IpcBridgeError> {
        let normalized = normalize_pipe_name(pipe_name)?;
        let wide_name = to_wide_null(&normalized);
        let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);

        let handle = loop {
            let h = unsafe {
                CreateFileW(
                    wide_name.as_ptr(),
                    GENERIC_WRITE,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    std::ptr::null_mut(),
                )
            };
            if h != INVALID_HANDLE_VALUE {
                break OwnedHandle(h);
            }

            let err = unsafe { GetLastError() };
            let now = Instant::now();
            if now >= deadline {
                return Err(std::io::Error::from_raw_os_error(err as i32).into());
            }
            let remaining = (deadline - now).as_millis().max(1) as u32;
            if err == ERROR_PIPE_BUSY {
                unsafe {
                    WaitNamedPipeW(wide_name.as_ptr(), remaining);
                }
            } else if err == ERROR_FILE_NOT_FOUND {
                thread::sleep(Duration::from_millis(15.min(remaining as u64)));
            } else {
                return Err(std::io::Error::from_raw_os_error(err as i32).into());
            }
        };

        let mut offset = 0usize;
        while offset < payload.len() {
            let chunk = (payload.len() - offset).min(64 * 1024) as u32;
            let mut written = 0u32;
            let ok = unsafe {
                WriteFile(
                    handle.0,
                    payload[offset..].as_ptr(),
                    chunk,
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            if written == 0 {
                return Err(IpcBridgeError::Protocol("pipe WriteFile wrote 0 bytes".into()));
            }
            offset += written as usize;
        }
        unsafe {
            FlushFileBuffers(handle.0);
        }
        Ok(())
    }
}

#[cfg(windows)]
pub use win_pipe::{write_to_named_pipe, NamedPipeServer};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc_bridge::{FrameKind, MemoryScanIngestor, ScanNode};
    use std::io::Cursor;
    use std::sync::Arc;

    fn sample_node(id: u64, parent_id: Option<u64>, name: &str, size: u64) -> ScanNode {
        ScanNode {
            id,
            parent_id,
            name: name.to_string(),
            full_path: format!(r"C:\Scan\{name}"),
            is_dir: parent_id.is_none(),
            logical_size: size,
            physical_size: size,
            modified_utc: "2026-11-18T12:00:00Z".to_string(),
            owner: String::new(),
            xxhash64: Some(id * 101),
            extension: if parent_id.is_none() {
                String::new()
            } else {
                ".dat".to_string()
            },
        }
    }

    #[test]
    fn ndjson_raw_nodes_batch_into_frames_and_complete() {
        let mut ndjson = String::new();
        for i in 1..=5 {
            let node = sample_node(
                i,
                if i == 1 { None } else { Some(1) },
                &format!("f{i}.dat"),
                i * 100,
            );
            ndjson.push_str(&serde_json::to_string(&node).unwrap());
            ndjson.push('\n');
        }

        let ingestor = MemoryScanIngestor::new();
        let config = StreamConfig {
            format: WireFormat::Auto,
            batch_size: 2,
            ..Default::default()
        };
        let summary =
            read_framed_stream(Cursor::new(ndjson.into_bytes()), &config, &ingestor).unwrap();

        assert_eq!(summary.total_nodes, 5);
        assert_eq!(summary.total_dirs, 1);
        assert_eq!(summary.total_files, 4);
        assert_eq!(summary.total_logical_bytes, 200 + 300 + 400 + 500);

        let frames = ingestor.frames();
        assert_eq!(frames.len(), 4);
        assert_eq!(frames[0].nodes.len(), 2);
        assert_eq!(frames[1].nodes.len(), 2);
        assert_eq!(frames[2].nodes.len(), 1);
        assert!(frames[3].is_final);
        assert_eq!(frames[3].frame_type, FrameKind::Complete);
    }

    #[test]
    fn ndjson_frame_envelopes_decode_including_error_frames() {
        let f1 = IpcStreamFrame {
            frame_type: FrameKind::Nodes,
            seq: 1,
            scan_root: r"C:\Env".into(),
            scanned_nodes: 1,
            total_bytes: 256,
            is_final: false,
            error: None,
            nodes: vec![sample_node(1, None, "Env", 256)],
        };
        let f2 = IpcStreamFrame {
            frame_type: FrameKind::Error,
            seq: 2,
            scan_root: r"C:\Env".into(),
            scanned_nodes: 1,
            total_bytes: 256,
            is_final: true,
            error: Some("partial failure".into()),
            nodes: vec![],
        };
        let wire = format!(
            "{}{}",
            encode_ndjson_frame(&f1).unwrap(),
            encode_ndjson_frame(&f2).unwrap()
        );
        let ingestor = MemoryScanIngestor::new();
        read_framed_stream(Cursor::new(wire.into_bytes()), &StreamConfig::default(), &ingestor)
            .unwrap();

        let frames = ingestor.frames();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[1].frame_type, FrameKind::Error);
        assert_eq!(frames[1].error.as_deref(), Some("partial failure"));
    }

    #[test]
    fn msgpack_length_prefixed_chunks_decode_cleanly() {
        let f1 = IpcStreamFrame {
            frame_type: FrameKind::Nodes,
            seq: 1,
            scan_root: r"D:\Data".into(),
            scanned_nodes: 2,
            total_bytes: 512,
            is_final: false,
            error: None,
            nodes: vec![
                sample_node(1, None, "Data", 0),
                sample_node(2, Some(1), "blob.dat", 512),
            ],
        };
        let f2 = IpcStreamFrame {
            frame_type: FrameKind::Complete,
            seq: 2,
            scan_root: r"D:\Data".into(),
            scanned_nodes: 2,
            total_bytes: 512,
            is_final: true,
            error: None,
            nodes: vec![],
        };

        let mut wire = encode_msgpack_chunk(&f1).unwrap();
        wire.extend_from_slice(&encode_msgpack_chunk(&f2).unwrap());

        let ingestor = MemoryScanIngestor::new();
        let summary =
            read_framed_stream(Cursor::new(wire), &StreamConfig::default(), &ingestor).unwrap();

        assert_eq!(summary.total_nodes, 2);
        assert_eq!(summary.total_files, 1);
        assert_eq!(summary.total_logical_bytes, 512);
        assert_eq!(ingestor.frames().len(), 2);
        assert_eq!(ingestor.nodes().len(), 2);
        assert_eq!(ingestor.nodes()[1].xxhash64, Some(202));
    }

    #[cfg(windows)]
    #[test]
    fn real_windows_named_pipe_round_trip() {
        let pipe_name = format!(r"\\.\pipe\wds_test_{}", std::process::id());
        let ingestor = Arc::new(MemoryScanIngestor::new());
        let server = NamedPipeServer::start(
            &pipe_name,
            StreamConfig {
                format: WireFormat::Auto,
                batch_size: 2,
                ..Default::default()
            },
            Arc::clone(&ingestor),
        )
        .unwrap();

        let frame1 = IpcStreamFrame {
            frame_type: FrameKind::Nodes,
            seq: 1,
            scan_root: r"C:\PipeDemo".into(),
            scanned_nodes: 2,
            total_bytes: 2048,
            is_final: false,
            error: None,
            nodes: vec![
                sample_node(1, None, "PipeDemo", 0),
                sample_node(2, Some(1), "clip.dat", 2048),
            ],
        };
        let frame2 = IpcStreamFrame {
            frame_type: FrameKind::Complete,
            seq: 2,
            scan_root: r"C:\PipeDemo".into(),
            scanned_nodes: 2,
            total_bytes: 2048,
            is_final: true,
            error: None,
            nodes: vec![],
        };

        let mut payload = encode_msgpack_chunk(&frame1).unwrap();
        payload.extend_from_slice(&encode_msgpack_chunk(&frame2).unwrap());

        write_to_named_pipe(&pipe_name, &payload, 3000).unwrap();

        for _ in 0..100 {
            if ingestor.summary().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        server.stop().unwrap();

        let summary = ingestor.summary().expect("pipe server should emit summary");
        assert_eq!(summary.total_nodes, 2);
        assert_eq!(summary.total_logical_bytes, 2048);
        assert_eq!(ingestor.nodes()[1].name, "clip.dat");
    }

    #[cfg(windows)]
    #[test]
    fn named_pipe_server_stops_cleanly_without_clients() {
        let pipe_name = format!(r"\\.\pipe\wds_test_stop_{}", std::process::id());
        let ingestor = Arc::new(MemoryScanIngestor::new());
        let server = NamedPipeServer::start(&pipe_name, StreamConfig::default(), ingestor).unwrap();
        assert_eq!(server.pipe_name(), pipe_name);
        server.stop().unwrap();
    }

    #[test]
    fn decodes_cpp_compatible_msgpack_and_raw_length_prefix() {
        // Exact MessagePack layout emitted by C++ IpcStreamer::SerializeFrameMessagePack
        // for a 1-node Nodes frame, verifying C++ -> Rust binary wire compatibility.
        let frame = IpcStreamFrame {
            frame_type: FrameKind::Nodes,
            seq: 1,
            scan_root: r"C:\Test".into(),
            scanned_nodes: 1,
            total_bytes: 4096,
            is_final: false,
            error: None,
            nodes: vec![ScanNode {
                id: 1,
                parent_id: None,
                name: "a.bin".into(),
                full_path: r"C:\Test\a.bin".into(),
                is_dir: false,
                logical_size: 4096,
                physical_size: 4096,
                modified_utc: "2026-11-18T12:00:00Z".into(),
                owner: String::new(),
                xxhash64: Some(0x1122_3344_5566_7788),
                extension: ".bin".into(),
            }],
        };

        let payload = rmp_serde::to_vec_named(&frame).unwrap();
        let mut raw_len_wire = (payload.len() as u32).to_le_bytes().to_vec();
        raw_len_wire.extend_from_slice(&payload);

        let ingestor = MemoryScanIngestor::new();
        let config = StreamConfig {
            format: WireFormat::MessagePack,
            ..Default::default()
        };
        let summary = read_framed_stream(Cursor::new(raw_len_wire), &config, &ingestor).unwrap();
        assert_eq!(summary.total_nodes, 1);
        assert_eq!(ingestor.nodes()[0].xxhash64, Some(0x1122_3344_5566_7788));
    }
}
