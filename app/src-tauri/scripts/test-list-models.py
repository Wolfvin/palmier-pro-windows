#!/usr/bin/env python3
"""Manual DoD test for Issue #6 fix — verifies list_models reads from
generation::models and returns loaded:true with non-empty arrays.

Compares the field shape against resources/read palmier://models/{video,image}
to confirm the only difference is the `type` field (present in list_models,
absent in resources/read).
"""
import json
from urllib.request import Request, urlopen

URL = "http://127.0.0.1:19789/mcp"


def call(method, params=None, id_=1):
    body = json.dumps({"jsonrpc": "2.0", "id": id_, "method": method, "params": params or {}})
    req = Request(URL, data=body.encode(), headers={"content-type": "application/json", "accept": "application/json, text/event-stream"})
    with urlopen(req) as r:
        return json.loads(r.read())


def step(n, msg):
    print(f"\n=== Step {n}: {msg} ===")


# Step 1: list_models type=video -> 5 models, loaded:true
step(1, "list_models type=video")
r = call("tools/call", {"name": "list_models", "arguments": {"type": "video"}}, id_="v1")
text = r["result"]["content"][0]["text"]
body = json.loads(text)
print(f"loaded={body['loaded']}, model_count={len(body['models'])}")
assert body["loaded"] is True, "loaded must be true"
assert len(body["models"]) == 5, f"expected 5 video models, got {len(body['models'])}"
for m in body["models"]:
    print(f"  - id={m['id']} type={m.get('type')} displayName={m['displayName']}")
    assert m["type"] == "video", f"expected type=video, got {m.get('type')}"
    for k in ["id", "displayName", "durations", "aspectRatios", "supportsFirstFrame", "supportsLastFrame", "supportsReferences"]:
        assert k in m, f"missing required field: {k}"

# Step 2: list_models type=image -> 3 models, loaded:true
step(2, "list_models type=image")
r = call("tools/call", {"name": "list_models", "arguments": {"type": "image"}}, id_="i1")
text = r["result"]["content"][0]["text"]
body = json.loads(text)
print(f"loaded={body['loaded']}, model_count={len(body['models'])}")
assert body["loaded"] is True
assert len(body["models"]) == 3, f"expected 3 image models, got {len(body['models'])}"
for m in body["models"]:
    print(f"  - id={m['id']} type={m.get('type')} displayName={m['displayName']}")
    assert m["type"] == "image"
    for k in ["id", "displayName", "aspectRatios", "supportsImageReference"]:
        assert k in m, f"missing required field: {k}"
ids = [m["id"] for m in body["models"]]
assert "nano-banana-pro" in ids, f"nano-banana-pro missing: {ids}"

# Step 3: list_models with no filter -> 8 models (5 video + 3 image)
step(3, "list_models (no filter)")
r = call("tools/call", {"name": "list_models", "arguments": {}}, id_="all")
text = r["result"]["content"][0]["text"]
body = json.loads(text)
print(f"loaded={body['loaded']}, total models={len(body['models'])}")
assert body["loaded"] is True
assert len(body["models"]) == 8, f"expected 8 models, got {len(body['models'])}"
vids = [m for m in body["models"] if m.get("type") == "video"]
imgs = [m for m in body["models"] if m.get("type") == "image"]
assert len(vids) == 5
assert len(imgs) == 3

# Step 4: cross-check field shape vs resources/read palmier://models/video
step(4, "field shape vs resources/read palmier://models/video")
r1 = call("resources/read", {"uri": "palmier://models/video"}, id_="r1")
rr_text = r1["result"]["contents"][0]["text"]
rr_body = json.loads(rr_text)
print(f"resources/read video models: {len(rr_body)}")
# Same model IDs.
lm_ids = {m["id"] for m in body["models"] if m.get("type") == "video"}
rr_ids = {m["id"] for m in rr_body}
assert lm_ids == rr_ids, f"model ID sets differ: lm={lm_ids} rr={rr_ids}"
# Compare field sets: list_models should have exactly one extra field (`type`).
lm_fields = set(json.loads(r["result"]["content"][0]["text"])["models"][0].keys())  # wrong - this is "all" not video; redo
r = call("tools/call", {"name": "list_models", "arguments": {"type": "video"}}, id_="v2")
lm_video_first = json.loads(r["result"]["content"][0]["text"])["models"][0]
rr_video_first = rr_body[0]
lm_field_set = set(lm_video_first.keys())
rr_field_set = set(rr_video_first.keys())
print(f"list_models fields:   {sorted(lm_field_set)}")
print(f"resources/read fields: {sorted(rr_field_set)}")
# resources/read = list_models minus the `type` field.
assert rr_field_set == lm_field_set - {"type"}, \
    f"field mismatch. rr={rr_field_set} lm={lm_field_set}"
# All OTHER field values must match.
for k in rr_field_set:
    assert lm_video_first[k] == rr_video_first[k], \
        f"field '{k}' differs: lm={lm_video_first[k]!r} rr={rr_video_first[k]!r}"
print("OK: list_models == resources/read + type field (video)")

# Step 5: same cross-check for image
step(5, "field shape vs resources/read palmier://models/image")
r1 = call("resources/read", {"uri": "palmier://models/image"}, id_="r2")
rr_text = r1["result"]["contents"][0]["text"]
rr_body = json.loads(rr_text)
r = call("tools/call", {"name": "list_models", "arguments": {"type": "image"}}, id_="i2")
lm_image_first = json.loads(r["result"]["content"][0]["text"])["models"][0]
rr_image_first = rr_body[0]
lm_field_set = set(lm_image_first.keys())
rr_field_set = set(rr_image_first.keys())
print(f"list_models fields:   {sorted(lm_field_set)}")
print(f"resources/read fields: {sorted(rr_field_set)}")
assert rr_field_set == lm_field_set - {"type"}
for k in rr_field_set:
    assert lm_image_first[k] == rr_image_first[k], \
        f"field '{k}' differs: lm={lm_image_first[k]!r} rr={rr_image_first[k]!r}"
print("OK: list_models == resources/read + type field (image)")

# Step 6: type=audio returns empty but loaded:true (audio catalog not ported)
step(6, "list_models type=audio (catalog not ported)")
r = call("tools/call", {"name": "list_models", "arguments": {"type": "audio"}}, id_="a1")
body = json.loads(r["result"]["content"][0]["text"])
print(f"loaded={body['loaded']}, audio models={len(body['models'])}")
assert body["loaded"] is True, "loaded must still be true (catalog layer is alive)"
assert len(body["models"]) == 0

print("\nALL DOD CHECKS PASSED")
