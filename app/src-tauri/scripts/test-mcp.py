#!/usr/bin/env python3
"""Manual DoD test for Worker #2 — wires the MCP server through JSON-RPC.

Verifies:
1. tools/list returns 31 tools
2. No tool returns "editor backend not yet wired up"
3. add_clips -> get_timeline shows the clip
4. undo reverts the state
"""
import json
import sys
from urllib.request import Request, urlopen

URL = "http://127.0.0.1:19789/mcp"


def call(method, params=None, id_=1):
    body = json.dumps({"jsonrpc": "2.0", "id": id_, "method": method, "params": params or {}})
    req = Request(URL, data=body.encode(), headers={"content-type": "application/json", "accept": "application/json, text/event-stream"})
    with urlopen(req) as r:
        return json.loads(r.read())


def tool(name, args=None):
    r = call("tools/call", {"name": name, "arguments": args or {}}, id_=name + "-call")
    text = r["result"]["content"][0]["text"]
    is_err = r["result"].get("isError", False)
    return text, is_err


def step(n, msg):
    print(f"\n=== Step {n}: {msg} ===")


# Step 1: tools/list — verify 31 tools
step(1, "tools/list (count, verify all 31)")
r = call("tools/list")
tools = r["result"]["tools"]
print(f"tool count: {len(tools)}")
assert len(tools) == 31, f"expected 31, got {len(tools)}"
for t in tools:
    print(f"  - {t['name']}")

# Step 2: import_media to seed an asset
step(2, "import_media (seed asset so add_clips can reference it)")
text, is_err = tool("import_media", {"source": {"url": "https://example.com/sample.mp4", "mimeType": "video/mp4"}, "name": "Sample video"})
print(f"isError={is_err}, body={text}")
assert not is_err, "import_media should succeed"

# Step 3: get_media
step(3, "get_media (verify asset registered)")
text, _ = tool("get_media")
media = json.loads(text)["media"]
print(f"media count: {len(media)}")
for a in media:
    print(f"  - id={a['id']} name={a['name']} type={a['type']}")
assert len(media) >= 1, "media list should be non-empty"

# Step 4: add_clips
step(4, "add_clips (place asset on timeline)")
text, is_err = tool("add_clips", {"entries": [{"mediaRef": "asset-1", "startFrame": 0, "durationFrames": 60}]})
print(f"isError={is_err}, body={text}")
assert not is_err, "add_clips should succeed"

# Step 5: get_timeline — clip should be there
step(5, "get_timeline (verify clip is on timeline)")
text, _ = tool("get_timeline")
tl = json.loads(text)
print(f"fps={tl['fps']}, totalFrames={tl['totalFrames']}, currentFrame={tl['currentFrame']}, canGenerate={tl['canGenerate']}")
print(f"tracks: {len(tl['tracks'])}")
all_clips = []
for t in tl["tracks"]:
    print(f"  - {t['label']} type={t['type']} clips={len(t['clips'])}")
    for c in t["clips"]:
        print(f"      clip id={c['id']} mediaRef={c['mediaRef']} startFrame={c['startFrame']} durationFrames={c['durationFrames']}")
        all_clips.append(c)
assert len(all_clips) == 1, f"expected exactly 1 clip on timeline, got {len(all_clips)}"
assert all_clips[0]["mediaRef"] == "asset-1", f"expected mediaRef=asset-1, got {all_clips[0]['mediaRef']}"
assert all_clips[0]["startFrame"] == 0
assert all_clips[0]["durationFrames"] == 60

# Step 6: undo
step(6, "undo (should revert add_clips)")
text, is_err = tool("undo")
print(f"isError={is_err}, body={text}")
assert not is_err, "undo should succeed"

# Step 7: get_timeline again — should show 0 tracks
step(7, "get_timeline again (verify undo reverted)")
text, _ = tool("get_timeline")
tl = json.loads(text)
print(f"tracks after undo: {len(tl['tracks'])}")
print(f"totalFrames after undo: {tl['totalFrames']}")
assert len(tl["tracks"]) == 0, f"expected 0 tracks after undo, got {len(tl['tracks'])}"
assert tl["totalFrames"] == 0

# Step 8: verify no tool returns "not yet wired up"
step(8, "verify all tools return real responses (no 'not yet wired up')")
not_wired_count = 0
for t in tools:
    name = t["name"]
    # Use minimal args — most tools will return a validation error (which is fine;
    # we just need to confirm they don't say "not yet wired up").
    text, is_err = tool(name, {})
    if "not yet wired up" in text.lower():
        not_wired_count += 1
        print(f"  FAIL: {name} still returns placeholder")
    else:
        # Truncate for display
        snippet = text[:80].replace("\n", " ")
        print(f"  OK: {name} -> isError={is_err}, body={snippet!r}")
assert not_wired_count == 0, f"{not_wired_count} tools still return placeholder"

print("\nALL DOD CHECKS PASSED")
