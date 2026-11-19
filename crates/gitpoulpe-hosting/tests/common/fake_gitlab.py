import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = sys.argv[1]
IDS = {"acme/platform/api": 100, "octo/api": 200, "octo/notes": 300}
BRANCHES = {"acme/platform/api": {"main", "feature/x", "feature/dup", "shared/br"}, "octo/api": {"main", "feature/y", "feature/z", "shared/br"},
            "octo/notes": {"main"}}
SHA = lambda c: c * 40


def mr(iid, state, source, target, source_project=100, title=None, draft=False, sha="a", **extra):
    base = {"iid": iid, "state": state, "title": title or f"MR {iid}", "draft": draft, "source_branch": source,
            "target_branch": target, "source_project_id": source_project, "target_project_id": 100,
            "author": {"username": "octo"}, "sha": SHA(sha), "description": f"Body {iid}",
            "web_url": f"https://gitlab.test/acme/platform/api/-/merge_requests/{iid}",
            "updated_at": f"2026-11-{10 + iid}T10:00:00Z",
            "diff_refs": {"base_sha": SHA("b"), "head_sha": SHA(sha), "start_sha": SHA("c")}}
    base.update(extra)
    return base


MRS = [mr(3, "merged", "feature/m", "main"), mr(4, "closed", "feature/c", "main"),
       mr(5, "opened", "feature/dup", "main", title="Draft: Work in progress", draft=True),
       mr(7, "opened", "feature/y", "main", source_project=200, sha="d"),
       mr(6, "opened", "shared/br", "main", source_project=200, sha="6"),
       mr(8, "opened", "feature/old", "main", sha="e"), mr(9, "opened", "feature/race", "main", sha="f")]
READS = {}


def diff_entry(path, text, **flags):
    entry = {"old_path": path, "new_path": path, "new_file": False, "renamed_file": False,
             "deleted_file": False, "diff": text}
    entry.update(flags)
    return entry


DIFFS = {
    7: [[diff_entry("src/a.rs", "@@ -1,2 +1,3 @@\n keep\n-old\n+new\n+more\n"),
         diff_entry("docs/new.md", "@@ -0,0 +1 @@\n+hello\n", new_file=True)],
        [diff_entry("img/logo.png", "", new_file=True),
         diff_entry("big.sql", "", too_large=True)]],
    5: [[diff_entry("one.txt", "@@ -1 +1 @@\n-a\n+b\n")]],
    8: [[diff_entry("legacy.txt", "@@ -1 +1 @@\n-a\n+b\n")]],
    9: [[diff_entry("race.txt", "@@ -1 +1 @@\n-a\n+b\n")]],
}
USERS = {
    "glgood": {"username": "Octo", "name": "Octo Cat", "scopes": ["read_api", "read_repository", "write_repository"],
               "expires_at": "2027-01-31"},
    "glnoexp": {"username": "Octo", "name": None, "scopes": ["api"], "expires_at": None},
    "glold": {"username": "Octo", "name": None, "scopes": None, "expires_at": None},
    "glslow": {"username": "Slow", "name": None, "scopes": ["api"], "expires_at": None},
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
        return self.projects_routes(path, token)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length).decode()
        auth = self.headers.get("Authorization", "")
        with open(LOG, "a") as f:
            f.write(json.dumps({"path": self.path, "auth": auth, "body": raw}) + "\n")
        token = auth.removeprefix("Bearer ").strip()
        if token not in USERS:
            return self.reply(401, {"message": "401 Unauthorized"})
        from urllib.parse import unquote
        route = self.path[len("/api/v4"):]
        if not (route.startswith("/projects/") and route.endswith("/merge_requests")):
            return self.reply(404, {"message": "404 Not Found"})
        full = unquote(route[len("/projects/"):-len("/merge_requests")])
        body = json.loads(raw or "{}")
        source_project = IDS.get(full)
        if source_project is None or body.get("source_branch") not in BRANCHES.get(full, set()):
            return self.reply(400, {"message": {"source_branch": ["is invalid"]}})
        target_project = body.get("target_project_id", source_project)
        if source_project != target_project and target_project != 100:
            return self.reply(422, {"message": "target project is not a parent"})
        for existing in MRS:
            if (existing["state"] == "opened" and existing["source_branch"] == body["source_branch"]
                    and existing["target_branch"] == body["target_branch"]
                    and existing["source_project_id"] == source_project):
                return self.reply(409, {"message": ["Another open merge request already exists for this source branch: !5"]})
        title = body["title"]
        created = mr(max(m["iid"] for m in MRS) + 1, "opened", body["source_branch"], body["target_branch"],
                     source_project=source_project, title=title, draft=title.startswith("Draft: "), sha="9",
                     description=body.get("description", ""))
        MRS.append(created)
        return self.reply(201, created)

    def project(self, full, **extra):
        namespace, _, name = full.rpartition("/")
        base = {"id": abs(hash(full)) % 10000, "path": name, "path_with_namespace": full,
                "namespace": {"full_path": namespace}, "description": None, "visibility": "private",
                "archived": False, "default_branch": "main",
                "http_url_to_repo": f"https://gitlab.test/{full}.git",
                "permissions": {"project_access": {"access_level": 40}, "group_access": None}}
        base.update(extra)
        return base

    def projects_routes(self, path, token):
        import time
        from urllib.parse import unquote
        route, _, query = path.partition("?")
        params = dict(p.split("=", 1) for p in query.split("&") if "=" in p)
        page = int(params.get("page", "1"))
        port = self.server.server_address[1]
        if token == "glslow":
            time.sleep(5)
            return self.reply(200, [self.project("slow/late")])
        if route == "/groups":
            return self.reply(200, [
                {"id": 3, "full_path": "acme/platform/infra", "description": "Infra"},
                {"id": 1, "full_path": "acme", "description": "Acme Inc"},
                {"id": 2, "full_path": "acme/platform", "description": None, "parent_id": 1}])
        if route == "/projects":
            items = [self.project("octo/notes", visibility="public"),
                     self.project("acme/platform/infra/tool"),
                     self.project("shared/lib", permissions={"project_access": None, "group_access": {"access_level": 20}},
                                  forked_from_project={"path_with_namespace": "upstream/lib"}),
                     self.project("octo/old", archived=True)]
            chunk = items[(page - 1) * 3: page * 3]
            headers = {}
            if page * 3 < len(items):
                headers["Link"] = f'<http://127.0.0.1:{port}/api/v4/projects?page={page + 1}>; rel="next"'
            return self.reply(200, chunk, headers)
        if route.startswith("/projects/"):
            return self.project_routes(route, params)
        if route.startswith("/groups/") and route.endswith("/projects"):
            group = unquote(route[len("/groups/"):-len("/projects")])
            if group == "acme/platform":
                return self.reply(200, [self.project("acme/platform/api"), self.project("acme/platform/infra/tool")])
            if group == "ghost":
                return self.reply(404, {"message": "404 Group Not Found"})
            return self.reply(200, [])
        return self.reply(404, {"message": "404 Not Found"})

    def project_routes(self, route, params):
        from urllib.parse import unquote
        rest = route[len("/projects/"):]
        enc, _, tail = rest.partition("/")
        full = unquote(enc)
        params = {k: unquote(v) for k, v in params.items()}
        if full not in IDS:
            return self.reply(404, {"message": "404 Project Not Found"})
        if tail == "":
            extra = {"id": IDS[full]}
            if full == "octo/api":
                extra["forked_from_project"] = {"path_with_namespace": "acme/platform/api"}
            return self.reply(200, self.project(full, **extra))
        if tail.startswith("repository/branches/"):
            branch = unquote(tail[len("repository/branches/"):])
            if branch in BRANCHES[full]:
                return self.reply(200, {"name": branch})
            return self.reply(404, {"message": "404 Branch Not Found"})
        if tail == "repository/compare":
            return self.compare(full, params)
        if tail == "merge_requests":
            items = [m for m in MRS if params.get("state", "all") in ("all", m["state"])
                     and params.get("source_branch", m["source_branch"]) == m["source_branch"]
                     and params.get("target_branch", m["target_branch"]) == m["target_branch"]]
            return self.reply(200, sorted(items, key=lambda m: m["updated_at"], reverse=True))
        if tail.startswith("merge_requests/"):
            return self.mr_detail(tail[len("merge_requests/"):], params)
        return self.reply(404, {"message": "404 Not Found"})

    def compare(self, full, params):
        src = params.get("from_project_id")
        cross = (full == "octo/api" and params["from"] == "main") or (full == "acme/platform/api" and params["from"] in ("feature/y", "feature/z"))
        if cross != (src is not None) or (src is not None and int(src) not in IDS.values()):
            return self.reply(404, {"message": "404 Not Found"})
        forward = params["from"] == "main"
        if full == "octo/api":
            count = 3
        elif params["from"] in ("feature/y", "feature/z"):
            count = 4
        else:
            count = 2 if forward else 1
        commits = [{"id": SHA("1"), "title": f"commit {i}"} for i in range(count)]
        diffs = [diff_entry("src/a.rs", "@@ -1 +1,2 @@\n-a\n+b\n+c\n"),
                 diff_entry("docs/new.md", "@@ -0,0 +1 @@\n+x\n", new_file=True)] if forward else []
        return self.reply(200, {"commits": commits, "diffs": diffs, "compare_timeout": False})

    def mr_detail(self, tail, params):
        iid_text, _, sub = tail.partition("/")
        found = next((m for m in MRS if str(m["iid"]) == iid_text), None)
        if found is None:
            return self.reply(404, {"message": "404 Not Found"})
        iid = found["iid"]
        port = self.server.server_address[1]
        if sub == "":
            READS[iid] = READS.get(iid, 0) + 1
            if iid == 9 and READS[iid] > 1:
                return self.reply(200, dict(found, sha=SHA("0")))
            return self.reply(200, found)
        if sub == "commits":
            return self.reply(200, [{"id": found["sha"], "title": "Second", "author_name": "Octo"},
                                    {"id": SHA("2"), "title": "First", "author_name": "Octo"}])
        if sub == "diffs":
            if iid == 8:
                return self.reply(404, {"message": "404 Not Found"})
            chunks = DIFFS[iid]
            page = int(params.get("page", "1"))
            headers = {}
            if page < len(chunks):
                headers["Link"] = (f'<http://127.0.0.1:{port}/api/v4/projects/acme%2Fplatform%2Fapi/merge_requests/'
                                   f'{iid}/diffs?page={page + 1}>; rel="next"')
            return self.reply(200, chunks[page - 1], headers)
        if sub == "changes" and iid == 8:
            return self.reply(200, dict(found, changes=DIFFS[8][0], overflow=True))
        return self.reply(404, {"message": "404 Not Found"})


server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
print(server.server_address[1], flush=True)
server.serve_forever()
