# Unix Socket Blocking Issue - Analysis and Recommendations

## Summary

When using `http get --unix-socket` to connect to a Unix socket that streams chunked responses (e.g., from `tail -f`), pressing Ctrl+C does not immediately terminate the command. Instead, nushell waits for the next chunk of data from the server before detecting the interrupt signal and exiting.

## Root Cause

The blocking occurs in the response body reading phase, specifically in the `Reader` implementation at:

**File: `/home/user/nushell/crates/nu-protocol/src/pipeline/byte_stream.rs`**

### The Problem

The `Reader` struct wraps the response stream and checks for interrupt signals (Ctrl+C) in two methods:

1. **`Reader::read()` (lines 816-820)**:
   ```rust
   fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
       self.signals.check(&self.span).map_err(ShellErrorBridge)?;
       self.reader.read(buf)  // This line blocks!
   }
   ```
   - Signal check happens BEFORE the read
   - If `self.reader.read(buf)` blocks waiting for data, Ctrl+C cannot be detected until the read completes

2. **`Reader::fill_buf()` (lines 824-830)**:
   ```rust
   fn fill_buf(&mut self) -> io::Result<&[u8]> {
       self.reader.fill_buf()  // No signal check at all!
   }
   ```
   - No signal check whatsoever
   - If the buffer is empty and needs to be filled from a blocking source, Ctrl+C is ignored

### Call Stack Leading to the Block

1. `http get` command → `crates/nu-command/src/network/http/get.rs:216`
2. `send_request_no_body()` → `crates/nu-command/src/network/http/client.rs:305`
3. `send_cancellable_request()` → `crates/nu-command/src/network/http/client.rs:555-590`
   - **This function handles Ctrl+C during the REQUEST phase** (while waiting for initial response)
   - It spawns a thread and polls with 100ms timeout, checking signals in the loop
4. `request_handle_response()` → `crates/nu-command/src/network/http/client.rs:893`
5. `response_to_buffer()` → `crates/nu-command/src/network/http/client.rs:170`
   - Creates a `ByteStream` with the response body reader
6. User consumes the ByteStream (e.g., pipes to output)
7. `ByteStream::reader()` → `crates/nu-protocol/src/pipeline/byte_stream.rs:489`
8. `Reader::read()` or `Reader::fill_buf()` → **BLOCKS HERE**

### Why It Blocks with Unix Sockets and Streaming Responses

When the server is streaming data (e.g., from `tail -f`):
- The server sends HTTP chunked transfer encoding
- Each chunk arrives only when new data is available (e.g., when a new line is written to the tailed file)
- The underlying `UnixStream::read()` call blocks indefinitely waiting for the next chunk
- Even though Ctrl+C is pressed, the signal check doesn't run until after the current `read()` completes
- Result: User must wait for the next line of output before the program exits

## Recommendations

### Option 1: Thread-Based Interruptible Read (Simplest, Consistent with Existing Pattern)

The simplest solution is to apply the same pattern used in `send_cancellable_request()` to the read operations. This maintains consistency with the existing codebase.

**Implementation approach:**
- Modify `Reader::read()` to spawn a thread that performs the blocking read
- Main thread polls with timeout (e.g., 100ms) and checks signals
- If Ctrl+C is detected, return an interrupted error immediately
- Similar pattern can be applied to `fill_buf()`

**Pros:**
- Consistent with existing `send_cancellable_request()` implementation
- Simple to implement
- Works on all platforms
- No external dependencies

**Cons:**
- Thread spawning overhead for every read operation (though this can be mitigated with a persistent reader thread)
- Slightly higher latency

**Code location to modify:**
- `crates/nu-protocol/src/pipeline/byte_stream.rs:816-830` (Reader impl)

### Option 2: Non-Blocking I/O with Polling (More Efficient, More Complex)

Use non-blocking I/O and poll/select to check for both data availability and signal status.

**Implementation approach:**
- Set the underlying socket/file descriptor to non-blocking mode
- Use a polling mechanism (e.g., `mio`, `polling` crate) to wait for data with timeout
- Check signals during each timeout iteration

**Pros:**
- More efficient (no thread spawning)
- Finer-grained control
- Industry standard approach

**Cons:**
- More complex implementation
- Requires handling non-blocking I/O state machine
- Platform-specific edge cases
- Requires additional dependencies

### Option 3: Read with Timeout in Loop (Middle Ground)

Set a read timeout on the underlying socket and retry in a loop while checking signals.

**Implementation approach:**
- Configure the underlying socket with a read timeout (e.g., 100ms)
- In `Reader::read()`, loop:
  - Check signals
  - Attempt read (will timeout after 100ms if no data)
  - If timeout, continue loop; if data or EOF, return; if error, return error

**Pros:**
- Simpler than full non-blocking I/O
- No thread spawning overhead
- Predictable behavior

**Cons:**
- Not all Read types support timeout configuration
- May need different implementations for different sources (File, UnixStream, etc.)
- Could impact throughput if timeout is too aggressive

## Recommended Implementation: Option 1 (Thread-Based)

**Rationale:**
1. **Consistency**: Matches the existing pattern in `send_cancellable_request()`
2. **Simplicity**: Straightforward to implement and understand
3. **Reliability**: Works across all types of Read sources
4. **No New Dependencies**: Uses only std library

### Specific Changes Needed

**File: `crates/nu-protocol/src/pipeline/byte_stream.rs`**

Create a new interruptible reader wrapper or modify the `Reader` implementation:

```rust
impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Check signals first (fast path for already-interrupted case)
        self.signals.check(&self.span).map_err(ShellErrorBridge)?;

        // If signals are empty (common case), just do the read
        if self.signals.is_empty() {
            return self.reader.read(buf);
        }

        // Otherwise, use interruptible read
        interruptible_read(&mut self.reader, buf, &self.signals, self.span)
    }
}

// Helper function similar to send_cancellable_request pattern
fn interruptible_read(
    reader: &mut dyn Read,
    buf: &mut [u8],
    signals: &Signals,
    span: Span,
) -> io::Result<usize> {
    let (tx, rx) = mpsc::channel::<io::Result<usize>>();

    // Perform read on background thread
    std::thread::spawn(move || {
        let result = reader.read(buf);
        let _ = tx.send(result);
    });

    // Poll for result while checking signals
    loop {
        signals.check(&span).map_err(ShellErrorBridge)?;

        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(result) => return result,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                return Err(io::Error::new(io::ErrorKind::Other, "reader thread disconnected"))
            }
        }
    }
}
```

**Note:** The above is pseudocode - the actual implementation needs to handle buffer ownership correctly since `buf` can't be moved into the thread. A proper implementation would use a channel to send owned data back, or use unsafe code with proper synchronization.

### Alternative Approach: Persistent Reader Thread

To avoid spawning a thread for every read, maintain a persistent reader thread per Reader instance that processes read requests from a channel. This is more complex but more efficient for high-frequency reads.

## Testing Plan

1. Set up a Unix socket server that streams chunked responses (as created in `test_socket_server.py`)
2. Start `http get --unix-socket /tmp/test_socket.sock http://localhost/`
3. Wait for first chunk to arrive
4. Press Ctrl+C
5. Verify that the command exits immediately (within ~100ms), not after the next chunk arrives

## Additional Notes

- The request phase already has proper Ctrl+C handling via `send_cancellable_request()`
- Only the response body reading phase has this issue
- This issue affects all streaming responses, not just Unix sockets
- However, it's most noticeable with Unix sockets doing slow streaming (like tail -f)
