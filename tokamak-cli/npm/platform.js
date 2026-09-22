"use strict";

const PACKAGES = {
  "darwin-arm64": "@tokamakdev/tok-darwin-arm64",
  "darwin-x64": "@tokamakdev/tok-darwin-x64",
  "linux-x64": "@tokamakdev/tok-linux-x64",
  "win32-x64": "@tokamakdev/tok-win32-x64",
};

function packageFor(platform, arch) {
  const name = PACKAGES[`${platform}-${arch}`];
  if (!name) throw new Error(`tokamak does not publish a CLI for ${platform} ${arch}`);
  return name;
}

function binaryPath(platform, arch, resolve) {
  const name = packageFor(platform, arch);
  const file = platform === "win32" ? "bin/tok.exe" : "bin/tok";
  try {
    return resolve(`${name}/${file}`);
  } catch {
    throw new Error(`${name} is not installed; reinstall @tokamakdev/tok with optional dependencies enabled`);
  }
}

module.exports = { packageFor, binaryPath };
