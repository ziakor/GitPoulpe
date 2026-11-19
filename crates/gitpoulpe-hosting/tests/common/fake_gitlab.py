import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = sys.argv[1]
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
        if route.startswith("/groups/") and route.endswith("/projects"):
            group = unquote(route[len("/groups/"):-len("/projects")])
            if group == "acme/platform":
                return self.reply(200, [self.project("acme/platform/api"), self.project("acme/platform/infra/tool")])
            if group == "ghost":
                return self.reply(404, {"message": "404 Group Not Found"})
            return self.reply(200, [])
        return self.reply(404, {"message": "404 Not Found"})


server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
print(server.server_address[1], flush=True)
server.serve_forever()
