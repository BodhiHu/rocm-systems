use crate::common::Time;

#[ctl_dsl]
mod mirage_ctl {
    /// Attach to a running process and optionally send data to its `stdin`.
    ///
    /// The server will read from [`stream`](Self::stream) and write to the
    /// exec's `stdin`. It responds with a stream of [`AttachReply`] messages
    /// carrying `stdout` / `stderr` chunks and eventually a [`RunExit`].
    attach({
        /// Identifier of the exec to attach to.
        exec_id: String,
    }) ... {
        /// Data to write to the exec's `stdin`.
        stream: Vec<u8>,
    } -> {
        /// Whether this chunk came from `stdout` or `stderr`.
        is_stdout: bool,
        /// Chunks of data read from the exec's `stdout` or `stderr`.
        output: Vec<u8>,
    } => {
        /// The exit code of the exec after it finishes.
        exit_code: i32,
    };

    /// Request the current simulated time for a session.
    time({
        /// session identifier to query the time for.
        session_id: String = None,
    }) -> {
        /// The current simulated time for the session, in nanoseconds. 
        session_time : Time
    } ? SessionNotFound;
}
