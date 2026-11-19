import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = sys.argv[1]
USERS = {
    "glgood": {"username": "Octo", "name": "Octo Cat", "scopes": ["read_api", "read_repository", "write_repository"],
               "expires_at": "2027-01-31"},
    "glnoexp": {"username": "Octo", "name": None, "scopes": ["api"], "expires_at": None},
    "glold": {"username": "Octo", "name": None, "scopes": None, "expires_at": None},
    "glother": {"username": "Someone", "name": None, "scopes": ["api"], "expires_at": None},
}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, status, body, headers=None):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        auth = self.headers.get("Authorization", "")
        token = auth.removeprefix("Bearer ").strip() or (self.headers.get("PRIVATE-TOKEN") or "")
        with open(LOG, "a") as f:
            f.write(json.dumps({"path": self.path, "auth": auth}) + "\n")
        if not self.path.startswith("/api/v4/"):
            return self.reply(404, {"message": "404 Not Found"})
        path = self.path[len("/api/v4"):]
        if token == "glrate":
            return self.reply(429, {"message": "Retry later"}, {"Retry-After": "30"})
        if token == "glforbid":
            return self.reply(403, {"message": "403 Forbidden - Your account is blocked"})
        user = USERS.get(token)
        if user is None:
            return self.reply(401, {"message": "401 Unauthorized"})
        if path == "/user":
            return self.reply(200, {"id": 1, "username": user["username"], "name": user["name"]})
        if path == "/personal_access_tokens/self":
            if user["scopes"] is None:
                return self.reply(404, {"message": "404 Not Found"})
            return self.reply(200, {"scopes": user["scopes"], "expires_at": user["expires_at"], "active": True})
        return self.reply(404, {"message": "404 Not Found"})


server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
print(server.server_address[1], flush=True)
server.serve_forever()
