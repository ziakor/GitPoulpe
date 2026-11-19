import json
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = sys.argv[1]
POLLS = [0]
FORKS = {}
SENT = []
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

    def pull_routes(self, rest, query):
        from urllib.parse import unquote
        params = dict(p.split("=", 1) for p in query.split("&") if "=" in p)
        parts = rest.split("/")
        repo = parts[0] + "/" + parts[1]
        if "/git/ref/heads/" in rest:
            branch = rest.split("/git/ref/heads/", 1)[1]
            if branch in ("feature/x", "wip", "feature/dup", "feature/bad"):
                return self.reply(200, {"ref": "refs/heads/" + branch})
            return self.reply(404, {"message": "Not Found"})
        if "/compare/" in rest:
            spec = unquote(rest.split("/compare/", 1)[1])
            base, head = spec.split("...", 1)
            if repo == "acme/proj" and not head.startswith("octo:") or repo == "octo/proj" and ":" in head:
                return self.reply(404, {"message": "Not Found"})
            return self.reply(200, {"ahead_by": 2, "behind_by": 1, "files": [
                {"filename": "src/a.rs", "status": "modified", "additions": 3, "deletions": 1},
                {"filename": "README.md", "status": "added", "additions": 10, "deletions": 0}]})
        if rest.startswith("acme/proj/issues/") and rest.endswith("/comments"):
            return self.reply(200, [{"id": 1, "user": {"login": "octo"}, "body": "hello", "created_at": "t1"}]
                              + [c for c in SENT if c["kind"] == "issue"])
        if rest.startswith("acme/proj/pulls") and "head" not in params:
            return self.pull_read(rest, params)
        if rest.endswith("/pulls"):
            head = unquote(params.get("head", ""))
            if head.endswith(":feature/dup"):
                return self.reply(200, [{"number": 7, "html_url": f"https://example.test/{repo}/pull/7",
                                         "title": "dup", "state": "open", "draft": False,
                                         "head": {"ref": "feature/dup"}, "base": {"ref": "main"}}])
            return self.reply(200, [])
        return None

    def pr(self, n):
        spec = {1: ("open", None, "sha1"), 2: ("closed", None, "sha2"), 3: ("closed", "2026-01-01T00:00:00Z", "sha3"),
                4: ("open", None, "newsha"), 5: ("open", None, "sha5"), 6: ("open", None, "sha6")}[n]
        return {"number": n, "title": f"PR {n}", "state": spec[0], "merged_at": spec[1], "draft": n == 4,
                "html_url": f"https://example.test/acme/proj/pull/{n}", "updated_at": "2026-02-02T00:00:00Z",
                "user": {"login": f"author{n}"}, "body": f"body {n}",
                "head": {"ref": f"feature/{n}", "sha": spec[2]}, "base": {"ref": "main", "sha": "basesha"}}

    def pull_read(self, rest, params):
        import time
        parts = rest.split("/")
        port = self.server.server_address[1]
        if len(parts) == 3:
            state = params.get("state", "open")
            items = [self.pr(n) for n in range(1, 7)]
            if state == "open":
                items = [i for i in items if i["state"] == "open"]
            elif state == "closed":
                items = [i for i in items if i["state"] == "closed"]
            return self.reply(200, items)
        n = int(parts[3])
        if n == 6:
            return self.reply(404, {"message": "Not Found"})
        if n == 5 and len(parts) == 4:
            time.sleep(4)
        if len(parts) == 4:
            return self.reply(200, self.pr(n))
        if parts[4] == "commits":
            last = "oldsha" if n == 4 else self.pr(n)["head"]["sha"]
            return self.reply(200, [
                {"sha": "c0", "commit": {"message": "first\n\nbody", "author": {"name": "A"}}, "author": {"login": "authorx"}},
                {"sha": last, "commit": {"message": "second", "author": {"name": "B"}}, "author": None}])
        if parts[4] == "comments":
            base = [
                {"id": 10, "user": {"login": "reviewer"}, "body": "root", "path": "f1.rs", "line": 1, "side": "RIGHT",
                 "commit_id": "sha1", "created_at": "t2"},
                {"id": 11, "user": {"login": "octo"}, "body": "answer", "path": "f1.rs", "line": 1, "side": "RIGHT",
                 "commit_id": "sha1", "in_reply_to_id": 10, "created_at": "t3"},
                {"id": 12, "user": {"login": "reviewer"}, "body": "old", "path": "f2.rs", "line": None,
                 "original_line": 3, "side": "LEFT", "commit_id": "sha0", "created_at": "t0"}]
            return self.reply(200, base + [c for c in SENT if c["kind"] == "review"])
        if parts[4] == "files":
            page = int(params.get("page", "1"))
            files = [{"filename": f"f{i}.rs", "status": "modified", "additions": i, "deletions": 1,
                      "patch": f"@@ -1 +1 @@\n-old{i}\n+new{i}"} for i in range(1, 4)]
            chunk = files[(page - 1) * 2: page * 2]
            headers = {}
            if page * 2 < len(files):
                headers["Link"] = f'<http://127.0.0.1:{port}/repos/acme/proj/pulls/{n}/files?page={page + 1}>; rel="next"'
            return self.reply(200, chunk, headers)
        return self.reply(404, {"message": "Not Found"})

    def repo_get(self, token):
        rest, _, query = self.path[len("/repos/"):].partition("?")
        rest = rest.lower() if "/git/ref/" not in rest and "/compare/" not in rest else rest
        if rest.count("/") > 1:
            handled = self.pull_routes(rest, query)
            if handled is not None:
                return handled
        name = rest
        sources = {"acme/tool": None, "octo/taken": None, "octo/proj": None,
                   "acme/proj": None, "octo/forked": "acme/proj",
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

    def comment_post(self, body):
        parts = self.path.split("/")
        text = body.get("body", "")
        if parts[4] == "issues":
            c = {"kind": "issue", "id": 100 + len(SENT), "user": {"login": "Octo"}, "body": text, "created_at": "t9"}
        else:
            if parts[-3] == "comments" and parts[-1] == "replies":
                c = {"kind": "review", "id": 100 + len(SENT), "user": {"login": "Octo"}, "body": text,
                     "path": "f1.rs", "line": 1, "side": "RIGHT", "commit_id": "sha1",
                     "in_reply_to_id": int(parts[-2]), "created_at": "t9"}
            else:
                if not body.get("commit_id") or not body.get("path"):
                    return self.reply(422, {"message": "Validation Failed", "errors": [{"message": "path missing"}]})
                c = {"kind": "review", "id": 100 + len(SENT), "user": {"login": "Octo"}, "body": text,
                     "path": body["path"], "line": body.get("line"), "side": body.get("side"),
                     "commit_id": body["commit_id"], "start_line": body.get("start_line"), "created_at": "t9"}
        SENT.append(c)
        if "FLAKY" in text:
            self.send_response(500)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        self.reply(201, c)

    def pull_post(self, body):
        parts = self.path.split("/")
        repo = parts[2] + "/" + parts[3]
        if repo == "acme/locked":
            return self.reply(403, {"message": "Resource not accessible by personal access token"})
        if body.get("head") == "feature/bad":
            return self.reply(422, {"message": "Validation Failed",
                                    "errors": [{"message": "No commits between main and feature/bad"}]})
        self.reply(201, {"number": 12, "html_url": f"https://example.test/{repo}/pull/12",
                         "title": body.get("title"), "state": "open", "draft": body.get("draft", False),
                         "head": {"ref": body.get("head")}, "base": {"ref": body.get("base")}})

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
        if self.path.startswith("/repos/acme/proj/") and "/comments" in self.path:
            return self.comment_post(body)
        if self.path.startswith("/repos/") and self.path.endswith("/pulls"):
            return self.pull_post(body)
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
