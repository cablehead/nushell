#!/usr/bin/env python3
import socket
import os
import subprocess
import sys
from http.server import BaseHTTPRequestHandler
import io

SOCKET_PATH = "/tmp/test_socket.sock"

class StreamingHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        """Handle GET request by tailing the test file"""
        self.send_response(200)
        self.send_header('Content-Type', 'text/plain')
        self.send_header('Transfer-Encoding', 'chunked')
        self.end_headers()

        # Start tailing the file
        process = subprocess.Popen(
            ['tail', '-f', '/tmp/test_output.txt'],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True
        )

        try:
            # Stream output from tail -f
            for line in process.stdout:
                # Send as chunked response
                chunk = line.encode('utf-8')
                self.wfile.write(f"{len(chunk):X}\r\n".encode())
                self.wfile.write(chunk)
                self.wfile.write(b"\r\n")
                self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError) as e:
            print(f"Connection closed by client: {e}", file=sys.stderr)
        finally:
            process.terminate()
            process.wait()

    def log_message(self, format, *args):
        # Custom logging
        sys.stderr.write(f"[SERVER] {format % args}\n")

def main():
    # Remove socket if it already exists
    if os.path.exists(SOCKET_PATH):
        os.unlink(SOCKET_PATH)

    # Create Unix socket
    server_socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server_socket.bind(SOCKET_PATH)
    server_socket.listen(1)

    print(f"Server listening on {SOCKET_PATH}", file=sys.stderr)

    try:
        while True:
            conn, _ = server_socket.accept()
            print("Client connected", file=sys.stderr)

            # Wrap socket in file-like objects for HTTP handler
            rfile = conn.makefile('rb')
            wfile = conn.makefile('wb')

            handler = StreamingHandler(rfile, conn.getpeername(), None)
            handler.rfile = rfile
            handler.wfile = wfile
            handler.connection = conn
            handler.handle_one_request()

            conn.close()
            print("Client disconnected", file=sys.stderr)
    except KeyboardInterrupt:
        print("\nShutting down server...", file=sys.stderr)
    finally:
        server_socket.close()
        if os.path.exists(SOCKET_PATH):
            os.unlink(SOCKET_PATH)

if __name__ == '__main__':
    main()
