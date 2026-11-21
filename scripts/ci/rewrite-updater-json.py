#!/usr/bin/env python3
"""Rewrite Tauri's latest.json to public release download URLs.

tauri-action writes ``https://api.github.com/repos/.../releases/assets/<id>``
URLs into the updater manifest. Those requests consume the unauthenticated
GitHub API quota (60/hour per IP) and return HTTP 403 once it is exhausted,
which breaks the in-app updater. Public release download URLs serve the same
assets without using the API.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import tempfile
from pathlib import Path

ASSET_URL = re.compile(r"/releases/assets/(\d+)$")


def rewrite(payload: dict, assets: dict[str, str], repo: str, tag: str) -> dict:
    base = f"https://github.com/{repo}/releases/download/{tag}/"
    for platform, info in payload.get("platforms", {}).items():
        match = ASSET_URL.search(info.get("url", ""))
        if match is None:
            continue
        name = assets.get(match.group(1))
        if name is None:
            raise SystemExit(f"no asset name for {platform}: {info['url']}")
        info["url"] = base + name
    remaining = [
        info["url"]
        for info in payload.get("platforms", {}).values()
        if "api.github.com" in info.get("url", "")
    ]
    if remaining:
        raise SystemExit(f"latest.json still has API URLs: {remaining}")
    return payload


def main() -> None:
    repo = os.environ["REPO"]
    tag = os.environ["TAG"]
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        subprocess.run(
            [
                "gh", "release", "download", tag, "--repo", repo,
                "--pattern", "latest.json", "--dir", str(work), "--clobber",
            ],
            check=True,
        )
        listing = subprocess.check_output(
            [
                "gh", "api", f"repos/{repo}/releases/tags/{tag}",
                "--jq", '.assets[] | "\\(.id) \\(.name)"',
            ],
            text=True,
        )
        assets: dict[str, str] = {}
        for line in listing.splitlines():
            if line.strip():
                asset_id, name = line.split(" ", 1)
                assets[asset_id] = name

        path = work / "latest.json"
        payload = json.loads(path.read_text())
        rewrite(payload, assets, repo, tag)
        path.write_text(json.dumps(payload, indent=2) + "\n")
        subprocess.run(
            ["gh", "release", "upload", tag, str(path), "--repo", repo, "--clobber"],
            check=True,
        )


if __name__ == "__main__":
    main()
