import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = sys.argv[1]
IDS = {"acme/platform/api": 100, "octo/api": 200, "octo/notes": 300, "acme/platform/dev": 400}
BRANCHES = {"acme/platform/api": {"main", "feature/x", "feature/dup", "shared/br"}, "octo/api": {"main", "feature/y", "feature/z", "shared/br"},
            "octo/notes": {"main"}, "acme/platform/dev": {"main"}}
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


def rmr(iid, sha, status="mergeable", project=100, pipeline="success", **extra):
    base = mr(iid, "opened", f"feature/r{iid}", "main", source_project=project, sha=sha,
              detailed_merge_status=status, has_conflicts=status == "conflict",
              blocking_discussions_resolved=status != "discussions_not_resolved",
              head_pipeline=None if pipeline is None else {"status": pipeline, "sha": SHA(sha)})
    base["target_project_id"] = 400 if project == 400 else 100
    base.update(extra)
    return base


RMRS = [rmr(20, "2"), rmr(21, "3", "not_approved"), rmr(22, "4", "conflict"),
        rmr(23, "5", "ci_still_running", pipeline="running"), rmr(24, "7", "need_rebase"),
        rmr(25, "8"), rmr(27, "1"), rmr(28, "a"), rmr(31, "b", "ci_must_pass", pipeline="failed"),
        rmr(32, "c", "ci_must_pass", pipeline=None), rmr(33, "d", "requested_changes"),
        rmr(34, "e", pipeline="success", source_project_id=200), rmr(40, "f", project=400, pipeline=None),
        rmr(41, "9", "checking")]
for item in RMRS:
    if item["iid"] == 34:
        item["source_project_id"] = 200
APPROVALS = {21: (2, ["alice"]), 22: (0, ["alice"]), 20: (0, []), 41: (1, [])}
NOTE_IDS = [1000]
DISC = {20: [
    {"id": "g1", "individual_note": True, "notes": [
        {"id": 501, "type": None, "system": False, "body": "General remark", "author": {"username": "octo2"},
         "created_at": "2026-11-12T09:00:00Z"}]},
    {"id": "d1", "individual_note": False, "notes": [
        {"id": 601, "type": "DiffNote", "system": False, "body": "Fix this", "author": {"username": "octo2"},
         "created_at": "2026-11-12T09:01:00Z",
         "position": {"new_path": "src/a.rs", "old_path": "src/a.rs", "new_line": 3, "old_line": None,
                      "head_sha": SHA("2")}},
        {"id": 602, "type": "DiffNote", "system": False, "body": "Done", "author": {"username": "Octo"},
         "created_at": "2026-11-12T09:02:00Z",
         "position": {"new_path": "src/a.rs", "old_path": "src/a.rs", "new_line": 3, "old_line": None,
                      "head_sha": SHA("2")}}]},
    {"id": "d2", "individual_note": False, "notes": [
        {"id": 603, "type": "DiffNote", "system": False, "body": "Old", "author": {"username": "octo2"},
         "created_at": "2026-11-12T09:03:00Z",
         "position": {"new_path": "src/a.rs", "old_path": "src/a.rs", "new_line": None, "old_line": 2,
                      "head_sha": SHA("0")}}]},
    {"id": "s1", "individual_note": True, "notes": [
        {"id": 700, "type": None, "system": True, "body": "added 1 commit", "author": {"username": "octo2"},
         "created_at": "2026-11-12T09:04:00Z"}]}]}
DRAFTS = {}


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
    20: [[diff_entry("docs/readme.md", "@@ -1 +1 @@\n-x\n+y\n"),
          diff_entry("src/a.rs", "@@ -1,3 +1,4 @@\n keep\n-old\n+new\n+more\n tail\n")]],
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
        data = b"" if status == 204 else json.dumps(body).encode()
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
        self.write("POST")

    def do_PUT(self):
        self.write("PUT")

    def do_DELETE(self):
        self.write("DELETE")

    def write(self, method):
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length).decode()
        auth = self.headers.get("Authorization", "")
        with open(LOG, "a") as f:
            f.write(json.dumps({"method": method, "path": self.path, "auth": auth, "body": raw}) + "\n")
        token = auth.removeprefix("Bearer ").strip()
        if token not in USERS:
            return self.reply(401, {"message": "401 Unauthorized"})
        from urllib.parse import unquote
        route = self.path[len("/api/v4"):]
        if "/merge_requests/" in route:
            return self.mr_write(method, route, json.loads(raw or "{}"))
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
        if full.isdigit():
            full = next((k for k, v in IDS.items() if v == int(full)), full)
        params = {k: unquote(v) for k, v in params.items()}
        if full not in IDS:
            return self.reply(404, {"message": "404 Project Not Found"})
        if tail == "":
            extra = {"id": IDS[full]}
            if full == "octo/api":
                extra["forked_from_project"] = {"path_with_namespace": "acme/platform/api"}
                extra.update(merge_method="merge", squash_option="always")
            if full == "acme/platform/api":
                extra.update(merge_method="merge", squash_option="default_off",
                             only_allow_merge_if_pipeline_succeeds=True)
            if full == "acme/platform/dev":
                extra.update(merge_method="ff", squash_option="default_on",
                             only_allow_merge_if_pipeline_succeeds=False,
                             permissions={"project_access": {"access_level": 30}, "group_access": None})
            if full == "octo/notes":
                extra.update(merge_method="rebase_merge", squash_option="never",
                             only_allow_merge_if_pipeline_succeeds=False)
            return self.reply(200, self.project(full, **extra))
        if tail.startswith("protected_branches/"):
            if full == "octo/notes":
                return self.reply(403, {"message": "403 Forbidden"})
            level = {"acme/platform/api": 30, "acme/platform/dev": 40}.get(full)
            if level is None:
                return self.reply(404, {"message": "404 Not Found"})
            return self.reply(200, {"name": "main", "merge_access_levels": [{"access_level": level}]})
        if tail.startswith("pipelines/") and tail.endswith("/jobs"):
            return self.jobs(tail, params)
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
        found = next((m for m in MRS + RMRS if str(m["iid"]) == iid_text), None)
        if found is None:
            return self.reply(404, {"message": "404 Not Found"})
        iid = found["iid"]
        port = self.server.server_address[1]
        if sub == "":
            READS[iid] = READS.get(iid, 0) + 1
            if iid == 9 and READS[iid] > 1:
                return self.reply(200, dict(found, sha=SHA("0")))
            return self.reply(200, found)
        if sub == "discussions":
            return self.reply(200, DISC.get(iid, []))
        if sub == "approvals":
            required, who = APPROVALS.get(iid, (0, []))
            return self.reply(200, {"approvals_required": required,
                                    "approved_by": [{"user": {"username": u, "id": 5}} for u in who]})
        if sub == "pipelines":
            shas = {20: [900], 34: [901]}.get(iid, [])
            pipes = [{"id": p, "sha": found["sha"], "status": "success",
                      "web_url": f"https://gitlab.test/p/{p}", "project_id": 200 if p == 901 else 100}
                     for p in shas]
            if iid == 21:
                pipes = [{"id": 902, "sha": SHA("0"), "status": "success", "web_url": "x", "project_id": 100}]
            return self.reply(200, pipes)
        if sub == "commits":
            return self.reply(200, [{"id": found["sha"], "title": "Second", "author_name": "Octo"},
                                    {"id": SHA("2"), "title": "First", "author_name": "Octo"}])
        if sub == "diffs":
            if iid == 8:
                return self.reply(404, {"message": "404 Not Found"})
            chunks = DIFFS.get(iid, [[diff_entry("f.txt", "@@ -1 +1 @@\n-a\n+b\n")]])
            page = int(params.get("page", "1"))
            headers = {}
            if page < len(chunks):
                headers["Link"] = (f'<http://127.0.0.1:{port}/api/v4/projects/acme%2Fplatform%2Fapi/merge_requests/'
                                   f'{iid}/diffs?page={page + 1}>; rel="next"')
            return self.reply(200, chunks[page - 1], headers)
        if sub == "changes" and iid == 8:
            return self.reply(200, dict(found, changes=DIFFS[8][0], overflow=True))
        return self.reply(404, {"message": "404 Not Found"})

    def jobs(self, tail, params):
        pid = int(tail[len("pipelines/"):-len("/jobs")])
        port = self.server.server_address[1]
        page = int(params.get("page", "1"))
        names = [("build", "success"), ("test", "failed"), ("lint", "manual"), ("deploy", "canceled"),
                 ("docs", "skipped"), ("e2e", "running")]
        jobs = [{"name": n, "status": st, "web_url": f"https://gitlab.test/j/{pid}/{n}"} for n, st in names]
        chunks = [jobs[:4], jobs[4:]] if pid == 900 else [jobs[:1]]
        headers = {}
        if page < len(chunks):
            headers["Link"] = f'<http://127.0.0.1:{port}/api/v4/projects/acme%2Fplatform%2Fapi/pipelines/{pid}/jobs?page={page + 1}>; rel="next"'
        return self.reply(200, chunks[page - 1], headers)

    def mr_write(self, method, route, body):
        from urllib.parse import unquote
        rest = route[len("/projects/"):]
        enc, _, tail = rest.partition("/merge_requests/")
        found_project = unquote(enc)
        iid_text, _, sub = tail.partition("/")
        found = next((m for m in MRS + RMRS if str(m["iid"]) == iid_text), None)
        if found is None or found_project not in IDS:
            return self.reply(404, {"message": "404 Not Found"})
        iid = found["iid"]
        author = {"username": "Octo"}
        note_id = lambda: NOTE_IDS.append(NOTE_IDS[-1] + 1) or NOTE_IDS[-1]

        def note(text, position=None):
            made = {"id": note_id(), "type": "DiffNote" if position else None, "system": False, "body": text,
                    "author": author, "created_at": "2026-11-19T10:00:00Z"}
            if position:
                made["position"] = position
            return made

        def valid_position(position):
            return (position.get("position_type") == "text" and position.get("head_sha") == found["sha"]
                    and position.get("new_path") and position.get("old_path")
                    and (position.get("new_line") or position.get("old_line")))

        if method == "POST" and sub == "notes":
            made = note(body["body"])
            DISC.setdefault(iid, []).append({"id": f"g{made['id']}", "individual_note": True, "notes": [made]})
            return self.reply(201, made)
        if method == "POST" and sub == "discussions":
            position = body.get("position")
            if position is not None and not valid_position(position):
                return self.reply(400, {"message": "400 Bad request - Note {:position=>[\"is invalid\"]}"})
            made = note(body["body"], position)
            disc = {"id": f"d{made['id']}", "individual_note": position is None, "notes": [made]}
            DISC.setdefault(iid, []).append(disc)
            return self.reply(201, disc)
        if method == "POST" and sub.startswith("discussions/") and sub.endswith("/notes"):
            disc = next((d for d in DISC.get(iid, []) if d["id"] == sub.split("/")[1]), None)
            if disc is None:
                return self.reply(404, {"message": "404 Discussion Not Found"})
            made = note(body["body"], disc["notes"][0].get("position"))
            disc["notes"].append(made)
            return self.reply(201, made)
        if method == "POST" and sub == "draft_notes":
            position = body.get("position")
            if body.get("note") == "boom" or (position is not None and not valid_position(position)):
                return self.reply(400, {"message": "400 Bad request"})
            draft = {"id": note_id(), "note": body["note"], "position": position}
            DRAFTS.setdefault(iid, []).append(draft)
            return self.reply(201, {"id": draft["id"], "note": draft["note"]})
        if method == "DELETE" and sub.startswith("draft_notes/"):
            DRAFTS[iid] = [d for d in DRAFTS.get(iid, []) if str(d["id"]) != sub.split("/")[1]]
            return self.reply(204, None)
        if method == "POST" and sub == "draft_notes/bulk_publish":
            for draft in DRAFTS.pop(iid, []):
                made = note(draft["note"], draft["position"])
                DISC.setdefault(iid, []).append({"id": f"d{made['id']}", "individual_note": draft["position"] is None,
                                                 "notes": [made]})
            return self.reply(204, None)
        if method == "POST" and sub == "approve":
            if iid == 25 or body.get("sha") != found["sha"]:
                return self.reply(401, {"message": "401 Unauthorized"})
            required, who = APPROVALS.get(iid, (0, []))
            APPROVALS[iid] = (required, who + ["Octo"])
            return self.reply(201, {"approved_by": []})
        if method == "PUT" and sub == "merge":
            if iid == 27:
                return self.reply(405, {"message": "405 Method Not Allowed"})
            if iid == 28:
                return self.reply(401, {"message": "401 Unauthorized"})
            if body.get("sha") != found["sha"]:
                return self.reply(409, {"message": "SHA does not match HEAD of source branch"})
            found["state"] = "merged"
            found["merge_commit_sha"] = SHA("m")
            return self.reply(200, found)
        return self.reply(404, {"message": "404 Not Found"})


server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
print(server.server_address[1], flush=True)
server.serve_forever()
