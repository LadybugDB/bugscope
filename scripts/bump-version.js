#!/usr/bin/env node
// Usage: node scripts/bump-version.js <new-version>
// Updates version in package.json and src-tauri/Cargo.toml

import { readFileSync, writeFileSync } from "fs";
import { resolve, dirname } from "path";
import { fileURLToPath } from "url";
import { execSync } from "child_process";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = resolve(__dirname, "..");

const newVersion = process.argv[2];
if (!newVersion) {
  console.error("Usage: node scripts/bump-version.js <new-version>");
  process.exit(1);
}

if (!/^\d+\.\d+\.\d+/.test(newVersion)) {
  console.error(`Invalid version format: "${newVersion}". Expected semver (e.g. 1.2.3)`);
  process.exit(1);
}

// Update package.json
const pkgPath = resolve(root, "package.json");
const pkg = JSON.parse(readFileSync(pkgPath, "utf8"));
const oldVersion = pkg.version;
pkg.version = newVersion;
writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
console.log(`package.json:             ${oldVersion} → ${newVersion}`);

// Update src-tauri/Cargo.toml
const cargoPath = resolve(root, "src-tauri", "Cargo.toml");
let cargo = readFileSync(cargoPath, "utf8");
cargo = cargo.replace(
  /^(version\s*=\s*)"[\d.]+"(\s*$)/m,
  `$1"${newVersion}"$2`
);
writeFileSync(cargoPath, cargo);
console.log(`src-tauri/Cargo.toml:     ${oldVersion} → ${newVersion}`);

// Update package-lock.json
try {
  execSync("npm install --package-lock-only --ignore-scripts", { cwd: root, stdio: "inherit" });
  console.log(`package-lock.json:        updated`);
} catch {
  console.warn("Warning: failed to update package-lock.json — run `npm install` manually");
}

// Update Cargo.lock
try {
  execSync(`cargo update --precise ${newVersion} --manifest-path src-tauri/Cargo.toml bugscope`, {
    cwd: root,
    stdio: "inherit",
  });
  console.log(`src-tauri/Cargo.lock:     updated`);
} catch {
  console.warn("Warning: failed to update Cargo.lock — run `cargo update` in src-tauri/ manually");
}

console.log(`\nVersion bumped to ${newVersion}`);
