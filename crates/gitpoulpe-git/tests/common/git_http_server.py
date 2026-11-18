import base64
import http.server
import os
import ssl
import subprocess
import sys

root, certfile, keyfile, auth, logfile = sys.argv[1:6]


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def handle_any(self):
        with open(logfile, "a") as log:
            sent = "yes" if self.headers.get("Authorization") else "no"
            log.write(f"{self.command} {self.path} auth={sent}\n")
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else b""
        if auth != "none":
            expected = "Basic " + base64.b64encode(auth.encode()).decode()
            if self.headers.get("Authorization") != expected:
                self.send_response(401)
                self.send_header("WWW-Authenticate", 'Basic realm="git"')
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
        path, _, query = self.path.partition("?")
        env = {
            "PATH": os.environ["PATH"],
            "GIT_PROJECT_ROOT": root,
            "GIT_HTTP_EXPORT_ALL": "1",
            "REQUEST_METHOD": self.command,
            "PATH_INFO": path,
            "QUERY_STRING": query,
            "CONTENT_TYPE": self.headers.get("Content-Type", ""),
            "CONTENT_LENGTH": str(len(body)),
            "REMOTE_USER": "u",
            "REMOTE_ADDR": "127.0.0.1",
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_CONFIG_SYSTEM": "/dev/null",
        }
        if self.headers.get("Content-Encoding"):
            env["HTTP_CONTENT_ENCODING"] = self.headers["Content-Encoding"]
        out = subprocess.run(["git", "http-backend"], input=body, env=env, capture_output=True).stdout
        head, _, payload = out.partition(b"\r\n\r\n")
        status, headers = "200 OK", []
        for line in head.split(b"\r\n"):
            key, _, value = line.partition(b":")
            if key.lower() == b"status":
                status = value.strip().decode()
            elif key:
                headers.append((key.decode(), value.strip().decode()))
        self.send_response(int(status.split()[0]))
        for key, value in headers:
            self.send_header(key, value)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    do_GET = handle_any
    do_POST = handle_any


class Server(http.server.ThreadingHTTPServer):
    def handle_error(self, request, client_address):
        pass


server = Server(("127.0.0.1", 0), Handler)
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(certfile, keyfile)
server.socket = context.wrap_socket(server.socket, server_side=True)
print("PORT", server.server_address[1], flush=True)
server.serve_forever()
