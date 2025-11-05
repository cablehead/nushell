#!/bin/bash

# Test script to reproduce the Unix socket blocking issue

echo "Starting Unix socket server in background..."
python3 test_socket_server.py &
SERVER_PID=$!
echo "Server PID: $SERVER_PID"

sleep 2

echo ""
echo "Starting nushell http get command..."
echo "After it starts receiving data, press Ctrl+C and observe the delay..."
echo ""

# Build nushell if needed
if [ ! -f "target/debug/nu" ]; then
    echo "Building nushell..."
    cargo build
fi

# Run the http get command
./target/debug/nu -c "http get --unix-socket /tmp/test_socket.sock http://localhost/"

echo ""
echo "Cleaning up..."
kill $SERVER_PID 2>/dev/null
rm -f /tmp/test_socket.sock
