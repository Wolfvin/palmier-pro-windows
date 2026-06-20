#!/usr/bin/env python3
"""Manual DoD test for Worker #3 — verifies resources/read returns non-empty
catalogs for palmier://models/video and palmier://models/image, with field
shapes matching the Swift VideoModelConfig / ImageModelConfig.
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


def step(n, msg):
    print(f"\n=== Step {n}: {msg} ===")


# Step 1: resources/list
step(1, "resources/list")
r = call("resources/list")
resources = r["result"]["resources"]
print(f"resource count: {len(resources)}")
for x in resources:
    print(f"  - uri={x['uri']} name={x['name']} mime={x['mimeType']}")
assert len(resources) == 2
uris = {x["uri"] for x in resources}
assert uris == {"palmier://models/video", "palmier://models/image"}, f"URIs changed: {uris}"

# Step 2: resources/read palmier://models/video
step(2, "resources/read palmier://models/video")
r = call("resources/read", {"uri": "palmier://models/video"}, id_=2)
text = r["result"]["contents"][0]["text"]
body = json.loads(text)
assert isinstance(body, list), f"expected list, got {type(body)}"
print(f"video models: {len(body)}")
assert len(body) >= 1, "video catalog must be non-empty"
for m in body:
    print(f"  - id={m['id']} displayName={m['displayName']} durations={m.get('durations')} aspectRatios={m.get('aspectRatios')} supportsFirstFrame={m.get('supportsFirstFrame')} supportsLastFrame={m.get('supportsLastFrame')} supportsReferences={m.get('supportsReferences')}")
    for opt in ["resolutions", "maxReferenceImages", "maxReferenceVideos", "maxReferenceAudios", "maxTotalReferences", "maxCombinedVideoRefSeconds", "maxCombinedAudioRefSeconds", "framesAndReferencesExclusive", "referenceTagNoun"]:
        if opt in m:
            print(f"      {opt}={m[opt]}")
    # Required fields per Swift videoModelInfo.
    for k in ["id", "displayName", "durations", "aspectRatios", "supportsFirstFrame", "supportsLastFrame", "supportsReferences"]:
        assert k in m, f"video model {m.get('id')} missing required field: {k}"
    # `type` must NOT appear in resources/read (it's only in list_models).
    assert "type" not in m, f"video model {m['id']} should NOT have type field in resources/read"

# Step 3: resources/read palmier://models/image
step(3, "resources/read palmier://models/image")
r = call("resources/read", {"uri": "palmier://models/image"}, id_=3)
text = r["result"]["contents"][0]["text"]
body = json.loads(text)
assert isinstance(body, list), f"expected list, got {type(body)}"
print(f"image models: {len(body)}")
assert len(body) >= 1, "image catalog must be non-empty"
for m in body:
    print(f"  - id={m['id']} displayName={m['displayName']} aspectRatios={m.get('aspectRatios')} supportsImageReference={m.get('supportsImageReference')}")
    for opt in ["resolutions", "qualities"]:
        if opt in m:
            print(f"      {opt}={m[opt]}")
    for k in ["id", "displayName", "aspectRatios", "supportsImageReference"]:
        assert k in m, f"image model {m.get('id')} missing required field: {k}"
    assert "type" not in m, f"image model {m['id']} should NOT have type field in resources/read"

# Step 4: resources/read unknown URI
step(4, "resources/read unknown URI")
r = call("resources/read", {"uri": "palmier://bogus"}, id_=4)
text = r["result"]["contents"][0]["text"]
print(f"unknown URI response: {text}")
assert "Unknown resource" in text, f"expected 'Unknown resource' fallback, got: {text}"

# Step 5: nano-banana-pro is in the image catalog (it's the canonical model
# ID referenced in the Swift source).
step(5, "verify nano-banana-pro present in image catalog")
r = call("resources/read", {"uri": "palmier://models/image"}, id_=5)
text = r["result"]["contents"][0]["text"]
body = json.loads(text)
ids = [m["id"] for m in body]
assert "nano-banana-pro" in ids, f"nano-banana-pro missing from image catalog: {ids}"
nano = next(m for m in body if m["id"] == "nano-banana-pro")
print(f"nano-banana-pro: {json.dumps(nano, indent=2)}")
# nano-banana-pro should have resolutions + qualities (per static catalog).
assert "resolutions" in nano
assert "qualities" in nano
assert nano["supportsImageReference"] is True

print("\nALL DOD CHECKS PASSED")
