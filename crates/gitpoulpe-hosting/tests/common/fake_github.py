import json
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = sys.argv[1]
POLLS = [0]
FORKS = {}
USERS = {
    "good": {"login": "Octo", "name": "Octo Cat", "scopes": "repo, read:org"},
    "fine": {"login": "Octo", "name": "Octo Cat", "scopes": None},
    "other": {"login": "Someone", "name": None, "scopes": "repo"},
    "good2": {"login": "Octo2", "name": None, "scopes": "repo"},
    "slow": {"login": "Slow", "name": None, "scopes": "repo"},
}


class Handler(BaseHTTPRequestHandler):
    host = "example.test"

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
                "clone_url": f"https://{self.host}/{owner}/{name}.git",
                "permissions": {"push": True}}
        base.update(extra)
        return base

    def listing(self, token):
        port = self.server.server_address[1]
        self.host = "two.test" if token == "good2" else "example.test"
        if token == "slow":
            import time
            time.sleep(6)
            return self.reply(200, [self.repo("slow", "late")])
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

    def repo_get(self, token):
        name = self.path[len("/repos/"):].split("?")[0].lower()
        sources = {"acme/tool": None, "octo/taken": None,
                   "octo/existing": "acme/existing", "acme/existing": None}
        if name in sources:
            extra = {"fork": True, "parent": {"full_name": sources[name]}} if sources[name] else {}
            return self.reply(200, self.repo(*name.split("/"), **extra))
        if name in FORKS:
            FORKS[name] += 1
            if FORKS[name] <= FORKS.get("lag", 0) + 1:
                return self.reply(404, {"message": "Not Found"})
            return self.reply(200, self.repo(*name.split("/"), fork=True,
                                             parent={"full_name": "acme/" + name.split("/")[1]}))
        self.reply(404, {"message": "Not Found"})

    def fork_post(self, body):
        parts = self.path.split("/")
        source = parts[2] + "/" + parts[3]
        dest = (body.get("organization") or "octo") + "/" + parts[3]
        if body.get("organization") == "locked":
            return self.reply(403, {"message": "You do not have permission to fork into this organization"})
        if dest in FORKS and not dest.endswith("/never"):
            return self.reply(202, self.repo(*dest.split("/")))
        FORKS[dest] = 0
        if parts[3] == "never":
            FORKS["lag"] = 99
        self.reply(202, self.repo(*dest.split("/"), fork=True, parent={"full_name": source}))

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length).decode()
        with open(LOG, "a") as f:
            f.write(json.dumps({"path": self.path, "auth": self.headers.get("Authorization"),
                                "accept": self.headers.get("Accept"), "body": raw}) + "\n")
        body = json.loads(raw or "{}")
        if self.path.startswith("/api/v3/"):
            self.path = self.path[len("/api/v3"):]
        if self.path.startswith("/repos/") and self.path.endswith("/forks"):
            return self.fork_post(body)
        if body.get("client_id") != "client-1":
            return self.reply(200, {"error": "incorrect_client_credentials"})
        if self.path == "/login/device/code":
            if body.get("scope") != "repo read:org":
                return self.reply(200, {"error": "bad_scope"})
            return self.reply(200, {"device_code": "dc-ok", "user_code": "ABCD-1234",
                                    "verification_uri": f"http://127.0.0.1:{self.server.server_address[1]}/login/device",
                                    "expires_in": 900, "interval": 5})
        if self.path == "/login/oauth/access_token":
            code = body.get("device_code")
            if body.get("grant_type") != "urn:ietf:params:oauth:grant-type:device_code":
                return self.reply(200, {"error": "unsupported_grant_type"})
            if code == "dc-deny":
                return self.reply(200, {"error": "access_denied"})
            if code == "dc-exp":
                return self.reply(200, {"error": "expired_token"})
            if code == "dc-off":
                return self.reply(200, {"error": "device_flow_disabled"})
            if code == "dc-ok":
                POLLS[0] += 1
                if POLLS[0] == 1:
                    return self.reply(200, {"error": "authorization_pending"})
                if POLLS[0] == 2:
                    return self.reply(200, {"error": "slow_down", "interval": 10})
                return self.reply(200, {"access_token": "good", "token_type": "bearer", "scope": "repo,read:org"})
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
        if self.path.startswith("/repos/"):
            return self.repo_get(token)
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
