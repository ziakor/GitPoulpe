import json
import struct
import sys
import threading
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = sys.argv[1]
POLLS = [0]
FORKS = {}
SENT = []
REVIEWS = []
MERGED = set()
U1 = "fd0fc972-c7df-42cc-b025-35deb408c0b6"
U2 = "0a0fbb9c-3fe6-443d-9621-749ebd906115"
U3 = "39be22d3-faa0-4641-b4a5-477dba83845c"
U4 = "4a497955-6e6f-4237-a2c8-585ac871ea28"


def png(width, height):
    raw = b"".join(b"\x00" + b"\xff\x00\x00\xff" * width for _ in range(height))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff)
    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


PNG = png(4, 2)
BEFORE = png(6, 3)
AFTER = png(8, 4)
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
        return True

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
            if repo == "acme/proj" and base == "basesha" and head == "sha7":
                return self.reply(200, {"merge_base_commit": {"sha": "mergebase7"}, "files": []})
            if repo == "acme/proj" and not head.startswith("octo:") or repo == "octo/proj" and ":" in head:
                return self.reply(404, {"message": "Not Found"})
            return self.reply(200, {"ahead_by": 2, "behind_by": 1, "files": [
                {"filename": "src/a.rs", "status": "modified", "additions": 3, "deletions": 1},
                {"filename": "README.md", "status": "added", "additions": 10, "deletions": 0}]})
        if rest == "acme/proj/branches/main/protection":
            return self.reply(200, {"required_status_checks": {"contexts": ["build", "lint", "missing"]},
                                    "required_pull_request_reviews": {"required_approving_review_count": 2}})
        if rest.startswith("acme/proj/contents/"):
            return self.contents(rest, params)
        if rest.startswith("acme/proj/commits/"):
            return self.checks_route(rest, params)
        if rest == "acme/proj/issues/7/comments":
            return self.reply(200, [self.with_image({"id": 70, "user": {"login": "octo"}, "created_at": "t1"}, "Écran")])
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

    def checks_route(self, rest, params):
        parts = rest.split("/")
        sha, kind = parts[3], parts[4]
        port = self.server.server_address[1]
        if sha == "sha5":
            return self.reply(500, {"message": "Server Error"})
        if kind == "check-runs":
            runs = []
            if sha == "sha1":
                runs = [
                    {"name": "build", "status": "completed", "conclusion": "success", "html_url": "https://ci.test/1"},
                    {"name": "lint", "status": "completed", "conclusion": "failure", "html_url": "https://ci.test/2"},
                    {"name": "deploy", "status": "in_progress", "conclusion": None, "html_url": "https://ci.test/3"},
                    {"name": "e2e", "status": "completed", "conclusion": "cancelled", "html_url": "https://ci.test/4"},
                    {"name": "docs", "status": "completed", "conclusion": "skipped", "html_url": "https://ci.test/5"}]
            page = int(params.get("page", "1"))
            chunk = runs[(page - 1) * 3: page * 3]
            headers = {}
            if page * 3 < len(runs):
                headers["Link"] = f'<http://127.0.0.1:{port}/repos/acme/proj/commits/{sha}/check-runs?page={page + 1}>; rel="next"'
            return self.reply(200, {"total_count": len(runs), "check_runs": chunk}, headers)
        if kind == "status":
            statuses = []
            if sha == "sha2":
                statuses = [{"context": "legacy/ci", "state": "pending", "target_url": "https://ci.test/old"}]
            if sha == "sha1":
                statuses = [{"context": "legacy/ok", "state": "success", "target_url": None}]
            return self.reply(200, {"state": "pending", "statuses": statuses})
        return self.reply(404, {"message": "Not Found"})

    def pr(self, n):
        spec = {1: ("open", None, "sha1"), 2: ("closed", None, "sha2"), 3: ("closed", "2026-01-01T00:00:00Z", "sha5"),
                4: ("open", None, "newsha"), 5: ("open", None, "sha5"), 6: ("open", None, "sha6"),
                7: ("open", None, "sha7")}[n]
        return {"number": n, "title": f"PR {n}", "state": spec[0], "merged_at": spec[1], "draft": n == 4,
                "html_url": f"https://example.test/acme/proj/pull/{n}", "updated_at": "2026-02-02T00:00:00Z",
                "user": {"login": f"author{n}"}, "body": self.attached_body() if n == 7 else f"body {n}",
                "head": {"ref": f"feature/{n}", "sha": spec[2]}, "base": {"ref": "main", "sha": "basesha"},
                "mergeable_state": {1: "clean", 4: "blocked"}.get(n, "unknown")} | (
                {"state": "closed", "merged_at": "2026-03-03T00:00:00Z"} if n in MERGED else {})

    def site(self):
        return f"http://127.0.0.1:{self.server.server_address[1]}"

    def rich(self):
        return "full+json" in (self.headers.get("Accept") or "")

    def attached_body(self):
        site = self.site()
        return (f'Avant :\n<img width="10" alt="Capture" src="{site}/user-attachments/assets/{U1}" />\n\n'
                f"{site}/user-attachments/assets/{U2}\n\n"
                f"[trace.zip]({site}/user-attachments/files/7/trace.zip) ![ext](https://evil.test/e.png) "
                f"![gone]({site}/user-attachments/assets/{U3})\nFin.")

    def attached_html(self):
        site = self.site()
        return (f'<p>Avant :<br><img alt="Capture" src="{site}/media/{U1}.png?jwt=t&amp;v=1"></p>'
                f'<details><summary><span class="m-1">demo.mp4</span></summary>'
                f'<video src="{site}/media/{U2}.mp4?jwt=t"></video></details>'
                f'<p><a href="x">trace.zip</a> <img src="{site}/camo/1" data-canonical-src="https://evil.test/e.png"> '
                f'<img src="{site}/media/{U3}.png?jwt=t"></p>')

    def with_image(self, item, text):
        item["body"] = f"{text} ![écran]({self.site()}/user-attachments/assets/{U4})"
        if self.rich():
            item["body_html"] = f'<p>{text} <img alt="écran" src="{self.site()}/media/{U4}.png?jwt=t"></p>'
        return item

    def contents(self, rest, params):
        if self.headers.get("Accept") != "application/vnd.github.raw+json":
            return self.reply(415, {"message": "raw media type expected"})
        path = rest.split("/contents/", 1)[1]
        versions = {("docs/old%20shot.png", "mergebase7"): BEFORE, ("docs/shot.png", "sha7"): AFTER}
        data = versions.get((path, params.get("ref")))
        if data is None:
            return self.reply(404, {"message": "Not Found"})
        self.send_response(200)
        self.send_header("Content-Type", "application/vnd.github.raw+json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)
        return True

    def media(self):
        path = self.path.partition("?")[0]
        if self.headers.get("Authorization"):
            return self.reply(400, {"message": "credentials sent to a media address"})
        if path == "/camo/1":
            self.send_response(302)
            self.send_header("Location", f"/media/{U1}.png?jwt=camo")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return True
        if path in (f"/media/{U1}.png", f"/media/{U4}.png"):
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", str(len(PNG)))
            self.end_headers()
            self.wfile.write(PNG)
            return True
        return self.reply(404, {"message": "Not Found"})

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
            item = self.pr(n)
            if n == 7 and self.rich():
                item["body_html"] = self.attached_html()
            return self.reply(200, item)
        if parts[4] == "commits":
            last = "oldsha" if n == 4 else self.pr(n)["head"]["sha"]
            return self.reply(200, [
                {"sha": "c0", "commit": {"message": "first\n\nbody", "author": {"name": "A"}}, "author": {"login": "authorx"}},
                {"sha": last, "commit": {"message": "second", "author": {"name": "B"}}, "author": None}])
        if parts[4] == "reviews":
            return self.reply(200, [{"id": 1, "user": {"login": "reviewer"}, "state": "APPROVED", "body": "ok",
                                     "commit_id": "sha1", "submitted_at": "t0"}] + [r for r in REVIEWS if r["pull"] == n])
        if parts[4] == "comments":
            base = [
                {"id": 10, "user": {"login": "reviewer"}, "body": "root", "path": "f1.rs", "line": 1, "side": "RIGHT",
                 "commit_id": "sha1", "created_at": "t2"},
                {"id": 11, "user": {"login": "octo"}, "body": "answer", "path": "f1.rs", "line": 1, "side": "RIGHT",
                 "commit_id": "sha1", "in_reply_to_id": 10, "created_at": "t3"},
                {"id": 12, "user": {"login": "reviewer"}, "body": "old", "path": "f2.rs", "line": None,
                 "original_line": 3, "side": "LEFT", "commit_id": "sha0", "created_at": "t0"}]
            if n == 7:
                base.append(self.with_image({"id": 71, "user": {"login": "reviewer"}, "path": "f1.rs", "line": 1,
                                             "side": "RIGHT", "commit_id": "sha7", "created_at": "t4"}, "zoom"))
            return self.reply(200, base + [c for c in SENT if c["kind"] == "review"])
        if parts[4] == "files":
            page = int(params.get("page", "1"))
            files = [{"filename": f"f{i}.rs", "status": "modified", "additions": i, "deletions": 1,
                      "patch": f"@@ -1 +1 @@\n-old{i}\n+new{i}"} for i in range(1, 4)]
            if n == 7:
                files += [{"filename": "docs/shot.png", "previous_filename": "docs/old shot.png", "status": "renamed",
                           "additions": 0, "deletions": 0},
                          {"filename": "assets/demo.mp4", "status": "added", "additions": 0, "deletions": 0}]
            chunk = files[(page - 1) * 2: page * 2]
            headers = {}
            if page * 2 < len(files):
                headers["Link"] = f'<http://127.0.0.1:{port}/repos/acme/proj/pulls/{n}/files?page={page + 1}>; rel="next"'
            return self.reply(200, chunk, headers)
        return self.reply(404, {"message": "Not Found"})

    def issue_item(self, number, title, state="open", labels=(), comments=0, pull=False, repo="acme/proj", author="reporter"):
        item = {"number": number, "title": title, "state": state, "user": {"login": author},
                "labels": [{"name": n} for n in labels], "comments": comments,
                "updated_at": f"2026-11-{10 + number % 10}T10:00:00Z",
                "html_url": f"https://github.test/{repo}/issues/{number}", "body": "It crashes.\nsecond line"}
        if pull:
            item["pull_request"] = {"url": "x"}
        return item

    def issue_routes(self, rest, query, token):
        parts = rest.split("/")
        if len(parts) < 3 or parts[2] != "issues":
            return None
        sub = parts[3:]
        if sub and sub[0].isdigit() and int(sub[0]) < 100:
            return None
        if token == "bad":
            return self.reply(401, {"message": "Bad credentials"})
        repo = "/".join(parts[:2])
        if repo == "acme/noissues":
            return self.reply(410, {"message": "Issues are disabled for this repo"})
        if repo == "acme/secret" and token == "limited":
            return self.reply(404, {"message": "Not Found"})
        if repo not in ("acme/proj", "acme/secret"):
            return self.reply(404, {"message": "Not Found"})
        params = dict(p.split("=", 1) for p in query.split("&") if "=" in p)
        port = self.server.server_address[1]
        if repo == "acme/secret":
            everything = [self.issue_item(201, "Private roadmap", repo=repo)]
            pages = [everything]
        else:
            everything = [self.issue_item(105, "Crash on start", labels=("bug",), comments=2),
                          self.issue_item(104, "A pull request", pull=True),
                          self.issue_item(103, "Docs typo", state="closed"),
                          self.issue_item(102, "Add dark mode", labels=("enhancement", "ui")),
                          self.issue_item(101, "Pagination bug")]
            pages = [everything[:3], everything[3:]]
        if not sub:
            state = params.get("state", "open")
            page = int(params.get("page", "1"))
            chunk = [i for i in pages[page - 1] if state == "all" or i["state"] == state]
            headers = {}
            if page < len(pages):
                headers["Link"] = f'<http://127.0.0.1:{port}/repos/{repo}/issues?state={state}&page={page + 1}>; rel="next"'
            return self.reply(200, chunk, headers)
        number = int(sub[0])
        found = next((i for i in everything if i["number"] == number), None)
        if found is None:
            return self.reply(404, {"message": "Not Found"})
        if len(sub) == 1:
            return self.reply(200, found)
        if sub[1] == "comments":
            return self.reply(200, [{"user": {"login": "dev"}, "body": "Can reproduce.", "created_at": "t1"},
                                    {"user": {"login": "lead"}, "body": "Fix coming.", "created_at": "t2"}])
        if sub[1] == "timeline":
            if token == "limited":
                return self.reply(403, {"message": "Resource not accessible"})
            return self.reply(200, [
                {"event": "labeled"},
                {"event": "cross-referenced", "source": {"issue": {"number": 12, "title": "Fix crash", "state": "open", "pull_request": {"url": "x"}}}},
                {"event": "cross-referenced", "source": {"issue": {"number": 12, "title": "Fix crash", "state": "open", "pull_request": {"url": "x"}}}},
                {"event": "cross-referenced", "source": {"issue": {"number": 40, "title": "Other issue", "state": "open"}}},
                {"event": "referenced", "commit_id": "abc1234def5678"},
                {"event": "closed", "commit_id": "abc1234def5678"},
                {"event": "closed", "commit_id": None}])
        return self.reply(404, {"message": "Not Found"})

    def search_issues(self, token, query):
        if token == "bad":
            return self.reply(401, {"message": "Bad credentials"})
        params = dict(p.split("=", 1) for p in query.split("&") if "=" in p)
        from urllib.parse import unquote_plus
        q = unquote_plus(params.get("q", ""))
        if "repo:acme/proj" not in q or "is:issue" not in q:
            return self.reply(422, {"message": "Validation Failed"})
        hits = [self.issue_item(102, "Add dark mode", labels=("enhancement", "ui"))] if "dark" in q else []
        return self.reply(200, {"total_count": len(hits), "items": hits})

    def repo_get(self, token):
        rest, _, query = self.path[len("/repos/"):].partition("?")
        rest = rest.lower() if "/git/ref/" not in rest and "/compare/" not in rest else rest
        issue = self.issue_routes(rest, query, token)
        if issue is not None:
            return issue
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
            if name == "acme/proj":
                extra = {"allow_merge_commit": True, "allow_squash_merge": True, "allow_rebase_merge": False}
            return self.reply(200, self.repo(*name.split("/"), **extra))
        if name in FORKS:
            FORKS[name] += 1
            if FORKS[name] <= FORKS.get("lag", 0) + 1:
                return self.reply(404, {"message": "Not Found"})
            return self.reply(200, self.repo(*name.split("/"), fork=True,
                                             parent={"full_name": "acme/" + name.split("/")[1]}))
        self.reply(404, {"message": "Not Found"})

    def review_post(self, body):
        parts = self.path.split("/")
        text = body.get("body", "")
        if parts[3] == "locked":
            return self.reply(403, {"message": "Resource not accessible by personal access token"})
        if body.get("event") == "APPROVE" and "self" in text:
            return self.reply(422, {"message": "Unprocessable Entity",
                                    "errors": ["Can not approve your own pull request"]})
        state = {"APPROVE": "APPROVED", "REQUEST_CHANGES": "CHANGES_REQUESTED", "COMMENT": "COMMENTED"}[body["event"]]
        review = {"pull": int(parts[-2]), "id": 50 + len(REVIEWS), "user": {"login": "Octo"}, "state": state, "body": text,
                  "commit_id": body["commit_id"], "submitted_at": "t9", "n_comments": len(body.get("comments", []))}
        REVIEWS.append(review)
        if "FLAKY" in text:
            self.send_response(502)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        self.reply(200, review)

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

    def do_PUT(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = json.loads(self.rfile.read(length).decode() or "{}")
        if self.path.startswith("/api/v3/"):
            self.path = self.path[len("/api/v3"):]
        with open(LOG, "a") as f:
            f.write(json.dumps({"path": self.path, "put": True, "body": body}) + "\n")
        parts = self.path.split("/")
        if not (self.path.endswith("/merge") and parts[2] == "acme"):
            return self.reply(404, {"message": "Not Found"})
        n = int(parts[5])
        if n == 1:
            if body.get("sha") != "sha1":
                return self.reply(409, {"message": "Head branch was modified. Review and try the merge again."})
            if "GHOST" not in body.get("commit_message", ""):
                MERGED.add(1)
            if "FLAKY" in body.get("commit_message", ""):
                self.send_response(502)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            return self.reply(200, {"merged": True, "sha": "mergesha", "message": "Pull Request successfully merged"})
        if n == 4:
            return self.reply(405, {"message": "Required status check \"lint\" is failing"})
        return self.reply(405, {"message": "Pull Request is not mergeable"})

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length).decode()
        with open(LOG, "a") as f:
            f.write(json.dumps({"path": self.path, "auth": self.headers.get("Authorization"),
                                "accept": self.headers.get("Accept"), "body": raw}) + "\n")
        body = json.loads(raw or "{}")
        if self.path.startswith("/api/v3/"):
            self.path = self.path[len("/api/v3"):]
        if self.path.startswith("/repos/") and self.path.endswith("/reviews"):
            return self.review_post(body)
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
            f.write(json.dumps({"path": self.path, "auth": self.headers.get("Authorization"),
                                "accept": self.headers.get("Accept")}) + "\n")
        if self.path.startswith(("/media/", "/camo/")):
            return self.media()
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
        if self.path.startswith("/search/issues"):
            return self.search_issues(token, self.path.partition("?")[2])
        if self.path.startswith("/notifications"):
            if token == "bad":
                return self.reply(401, {"message": "Bad credentials"})
            return self.reply(200, [
                {"id": "n1", "unread": True, "reason": "review_requested", "updated_at": "2026-11-18T10:00:00Z",
                 "subject": {"title": "PR 1", "type": "PullRequest",
                             "url": "https://api.github.com/repos/acme/proj/pulls/1"},
                 "repository": {"full_name": "acme/proj"}},
                {"id": "n2", "unread": True, "reason": "ci_activity", "updated_at": "2026-11-18T11:00:00Z",
                 "subject": {"title": "CI failed", "type": "CheckSuite", "url": "https://api.github.com/repos/acme/proj/check-suites/9"},
                 "repository": {"full_name": "acme/proj"}},
                {"id": "n3", "unread": False, "reason": "mention", "updated_at": "2026-11-18T12:00:00Z",
                 "subject": {"title": "Old", "type": "Issue", "url": "https://api.github.com/repos/acme/proj/issues/3"},
                 "repository": {"full_name": "acme/proj"}},
                {"id": "n4", "unread": True, "reason": "mention", "updated_at": "2026-11-18T13:00:00Z",
                 "subject": {"title": "Question", "type": "Issue", "url": "https://api.github.com/repos/acme/proj/issues/12"},
                 "repository": {"full_name": "acme/proj"}}])
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
