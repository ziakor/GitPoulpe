import json
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = sys.argv[1]
USERS = {
    "good": {"login": "Octo", "name": "Octo Cat", "scopes": "repo, read:org"},
    "fine": {"login": "Octo", "name": "Octo Cat", "scopes": None},
    "other": {"login": "Someone", "name": None, "scopes": "repo"},
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

    def repo(self, owner, name, **extra):
        base = {"name": name, "owner": {"login": owner}, "description": None, "private": False,
                "fork": False, "archived": False, "default_branch": "main",
                "clone_url": f"https://example.test/{owner}/{name}.git",
                "permissions": {"push": True}}
        base.update(extra)
        return base

    def listing(self, token):
        port = self.server.server_address[1]
        path, _, query = self.path.partition("?")
        params = dict(p.split("=", 1) for p in query.split("&") if "=" in p)
        page = int(params.get("page", "1"))
        if token == "sso":
            return self.reply(403, {"message": "SAML"}, {"X-GitHub-SSO": "required; url=https://github.com/orgs/acme/sso"})
        if token == "bad":
            return self.reply(401, {"message": "Bad credentials"})
        if path == "/user/orgs":
            return self.reply(200, [{"login": "Zeta", "description": "z"}, {"login": "acme", "description": None}])
        if path == "/orgs/ghost/repos":
            return self.reply(404, {"message": "Not Found"})
        if path == "/orgs/acme/repos":
            return self.reply(200, [self.repo("acme", "tool", archived=True, permissions={"push": False})])
        if path == "/user/repos":
            pages = {
                1: [self.repo("octo", "b", private=True, description="secret"), self.repo("acme", "Zed", fork=True)],
                2: [self.repo("octo", "a")],
            }
            headers = {}
            if page < 2:
                headers["Link"] = f'<http://127.0.0.1:{port}/user/repos?page={page + 1}>; rel="next"'
            if token == "evil" and page == 1:
                headers["Link"] = '<http://127.0.0.1:9/user/repos?page=2>; rel="next"'
            return self.reply(200, pages[page], headers)
        return self.reply(404, {"message": "Not Found"})

    def do_GET(self):
        with open(LOG, "a") as f:
            f.write(json.dumps({"path": self.path, "auth": self.headers.get("Authorization")}) + "\n")
        if self.path.startswith("/api/v3/"):
            self.path = self.path[len("/api/v3"):]
        auth = self.headers.get("Authorization", "")
        token = auth.removeprefix("Bearer ").strip()
        if self.path == "/redirect":
            self.send_response(302)
            self.send_header("Location", "http://127.0.0.1:9/steal")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        if self.path.startswith(("/user/orgs", "/user/repos", "/orgs/")):
            return self.listing(token)
        if self.path != "/user":
            return self.reply(404, {"message": "Not Found"})
        if token == "sso":
            return self.reply(403, {"message": "Resource protected by organization SAML enforcement."},
                              {"X-GitHub-SSO": "required; url=https://github.com/orgs/acme/sso?authorization_request=1"})
        if token == "limited":
            return self.reply(403, {"message": "API rate limit exceeded"},
                              {"X-RateLimit-Remaining": "0", "X-RateLimit-Reset": "1893456000"})
        if token == "denied":
            return self.reply(403, {"message": "Resource not accessible by personal access token"})
        user = USERS.get(token)
        if user is None:
            return self.reply(401, {"message": "Bad credentials"})
        headers = {}
        if user["scopes"] is not None:
            headers["X-OAuth-Scopes"] = user["scopes"]
        self.reply(200, {"login": user["login"], "name": user["name"]}, headers)


server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
print(server.server_address[1], flush=True)
server.serve_forever()
