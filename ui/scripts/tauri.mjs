#!/usr/bin/env node
// Runs the Tauri CLI (@tauri-apps/cli) against crates/mode-app. The repo does
// not use the `src-tauri` layout (08 §3.1), so the CLI is pointed at the crate
// and at ui/ as the frontend through its TAURI_APP_PATH / TAURI_FRONTEND_PATH
// variables. Cross-platform (no shell `VAR=... cmd` needed on Windows).
//
// `dev`, `build` and `bundle` first build the `mode-hook` sidecar
// (ui/scripts/build-hook.mjs) with the same profile and `--target`, because
// tauri.conf.json lists it under `bundle.externalBin`.
//
//   npm run tauri -- dev
//   npm run tauri -- build --bundles deb,appimage
//   npm run tauri -- build --debug --bundles deb
//   npm run tauri -- build --target x86_64-apple-darwin --bundles app,dmg
import { createRequire } from 'node:module'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { buildHook } from './build-hook.mjs'

const ui = resolve(dirname(fileURLToPath(import.meta.url)), '..')
process.env.TAURI_APP_PATH ??= resolve(ui, '..', 'crates', 'mode-app')
process.env.TAURI_FRONTEND_PATH ??= ui

/** The sidecar build matching a Tauri command line, or null when none is needed. */
function hookBuildFor(args) {
  const [command, ...rest] = args
  if (!['dev', 'build', 'bundle'].includes(command)) return null
  // Arguments after `--` go to cargo / the app, not to the Tauri CLI.
  const own = rest.includes('--') ? rest.slice(0, rest.indexOf('--')) : rest
  if (own.some((a) => a === '-h' || a === '--help')) return null
  let release = command !== 'dev'
  let target
  for (let i = 0; i < own.length; i++) {
    const a = own[i]
    if (a === '--debug' || a === '-d') release = false
    else if (a === '--release' || a === '-r') release = true
    else if (a === '--target' || a === '-t') target = own[++i]
    else if (a.startsWith('--target=')) target = a.slice('--target='.length)
  }
  return { release, target }
}

const args = process.argv.slice(2)
const hook = hookBuildFor(args)
if (hook) buildHook(hook)

const require = createRequire(import.meta.url)
const { run, logError } = require('@tauri-apps/cli')
run(args, 'npm run tauri --').catch((err) => {
  logError(err.message)
  process.exit(1)
})
