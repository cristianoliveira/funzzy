#!/usr/bin/env node

import { readFile } from "node:fs/promises";

const manifest = process.env.FZZ_TRIGGER_PATHS_FILE;

if (!manifest) {
  console.log("No changed-path batch (init or manual run).");
  process.exit(0);
}

const paths = JSON.parse(await readFile(manifest, "utf8"));

if (!Array.isArray(paths) || paths.some((path) => typeof path !== "string")) {
  throw new TypeError(
    "FZZ_TRIGGER_PATHS_FILE must contain a JSON array of strings",
  );
}

for (const path of paths) {
  console.log(JSON.stringify(path));
}
