#!/bin/bash
# Manual test to verify that Ctrl+C now works immediately with streaming Unix socket responses

echo "=== Unix Socket Ctrl+C Fix - Manual Test ==="
echo ""
echo "This test verifies that pressing Ctrl+C during a streaming HTTP response"
echo "now exits immediately instead of waiting for the next chunk of data."
echo ""

# Check if Python is available
if ! command -v python3 &> /dev/null; then
    echo "Error: python3 is required but not found"
    exit 1
fi

# Start the Unix socket server in the background
echo "Step 1: Starting Unix socket server..."
python3 ./test_socket_server.py &
SERVER_PID=$!
echo "Server PID: $SERVER_PID"

# Give the server time to start
sleep 2

# Create the test file and add initial content
echo "Step 2: Creating test file with initial content..."
echo "Initial line at $(date)" > /tmp/test_output.txt

# Verify nushell binary exists
if [ ! -f "./target/debug/nu" ]; then
    echo "Error: nushell binary not found. Please run: cargo build"
    kill $SERVER_PID 2>/dev/null
    exit 1
fi

echo ""
echo "Step 3: Starting nushell http get command..."
echo "=========================================="
echo "The command will connect and start streaming output."
echo ""
echo "INSTRUCTIONS:"
echo "1. Wait a moment for the first line to appear"
echo "2. Press Ctrl+C"
echo "3. Observe how quickly the command exits"
echo ""
echo "EXPECTED BEHAVIOR (AFTER FIX):"
echo "  - Command should exit within ~100ms of pressing Ctrl+C"
echo ""
echo "OLD BEHAVIOR (BEFORE FIX):"
echo "  - Command would wait until the next line was written to the file"
echo "  - This could be indefinite if no more lines were written"
echo ""
echo "Press Enter to start the test..."
read

echo "Starting HTTP GET request (press Ctrl+C to test interrupt handling)..."
echo "=========================================="
./target/debug/nu -c 'http get --unix-socket /tmp/test_socket.sock http://localhost/'

EXIT_CODE=$?
echo ""
echo "=========================================="
echo "Command exited with code: $EXIT_CODE"

if [ $EXIT_CODE -eq 0 ]; then
    echo "Note: Exit code 0 means the stream ended normally"
elif [ $EXIT_CODE -eq 130 ] || [ $EXIT_CODE -eq 1 ]; then
    echo "Note: Exit code $EXIT_CODE likely means Ctrl+C was pressed"
fi

echo ""
echo "Step 4: Cleanup..."
kill $SERVER_PID 2>/dev/null
rm -f /tmp/test_socket.sock
rm -f /tmp/test_output.txt

echo ""
echo "=== Test Complete ==="
echo ""
echo "If the command exited quickly (within ~100ms) after pressing Ctrl+C,"
echo "the fix is working correctly!"
