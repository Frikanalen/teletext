"""Client for the ttxd page API (docs/design.md#api)."""

import json
import urllib.error
import urllib.request


class TtxdError(Exception):
    pass


class Ttxd:
    def __init__(self, url: str, token: str, timeout: float = 5):
        self.url = url.rstrip("/")
        self.token = token
        self.timeout = timeout

    def _call(self, method: str, path: str, body: object = None) -> dict:
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(
            self.url + path,
            data=data,
            method=method,
            headers={
                "Authorization": f"Bearer {self.token}",
                "Content-Type": "application/json",
            },
        )
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                return json.load(resp)
        except urllib.error.HTTPError as e:
            try:
                msg = json.load(e).get("error", e.reason)
            except (ValueError, AttributeError):
                msg = e.reason
            raise TtxdError(f"{method} {path}: {e.code} {msg}") from None
        except (urllib.error.URLError, OSError, ValueError) as e:
            raise TtxdError(f"{method} {path}: {e}") from None

    def put_page(
        self,
        page: str,
        subpages: list[list[str]],
        *,
        ttl_secs: int,
        cycle_secs: int = 10,
        flash: bool = False,
    ) -> None:
        self._call(
            "PUT",
            f"/v1/pages/{page}",
            {
                "subpages": subpages,
                "cycle_secs": cycle_secs,
                "ttl_secs": ttl_secs,
                "flash": flash,
            },
        )

    def flash_rows(self, page: str, rows: dict[int, str], subpage: int = 0) -> None:
        body = {"rows": {str(n): text for n, text in rows.items()}, "subpage": subpage}
        self._call("POST", f"/v1/pages/{page}/flash", body)

    def live_pages(self) -> set[str]:
        """Pages ttxd is sending now (stored and not expired)."""
        pages = self._call("GET", "/v1/pages")["pages"]
        return {p["page"] for p in pages if not p.get("expired")}
