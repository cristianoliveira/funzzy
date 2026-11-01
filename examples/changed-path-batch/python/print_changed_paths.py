#!/usr/bin/env python3

import json
import os

manifest = os.environ.get("FZZ_TRIGGER_PATHS_FILE")

if not manifest:
    print("No changed-path batch (init or manual run).")
    raise SystemExit(0)

with open(manifest, encoding="utf-8") as manifest_file:
    paths = json.load(manifest_file)

valid_paths = isinstance(paths, list) and all(isinstance(path, str) for path in paths)
if not valid_paths:
    raise TypeError("FZZ_TRIGGER_PATHS_FILE must contain a JSON array of strings")

for path in paths:
    print(json.dumps(path, ensure_ascii=False))
