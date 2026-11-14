#!/usr/bin/env node
// Builds the `mode-hook` relay and places it where the Tauri bundler expects
// the sidecar (`bundle.externalBin: ["binaries/mode-hook"]` in
// crates/mode-app/tauri.conf.json): crates/mode-app/binaries/mode-hook-<triple>[.exe].
// The bundler ships it next to the app binary with the triple stripped
// (Contents/MacOS/mode-hook, /usr/bin/mode-hook, <install dir>\mode-hook.exe);
// at launch the app copies it to <app_data_dir>/bin/ (08 §7, 05 §3.6).
//
//   node ui/scripts/build-hook.mjs                     # release, host triple
//   node ui/scripts/build-hook.mjs --debug             # dev profile (what `tauri dev` uses)
//   node ui/scripts/build-hook.mjs --target x86_64-apple-darwin
//   node ui/scripts/build-hook.mjs --target universal-apple-darwin   # lipo of both Mac triples
//
// `ui/scripts/tauri.mjs` runs this before `tauri dev`, `tauri build` and
// `tauri bundle`, with the same profile and `--target` as the Tauri command,
// so the copy tauri-build places in target/<profile>/ is the relay cargo
// built for that profile. `--locked` is passed when CI is set.
import { spawnSync } from 'node:child_process'
import { chmodSync, copyFileSync, mkdirSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')
const outDir = join(root, 'crates', 'mode-app', 'binaries')

function fail(message) {
  console.error(`build-hook: ${message}`)
  process.exit(1)
}

function run(cmd, args, opts = {}) {
  const r = spawnSync(cmd, args, { cwd: root, encoding: 'utf8', ...opts })
  if (r.error) fail(`could not run ${cmd}: ${r.error.message}`)
  if (r.status !== 0) {
    if (opts.stdio !== 'inherit') process.stderr.write(r.stderr ?? '')
    fail(`${cmd} ${args.join(' ')} exited with ${r.status}`)
  }
  return r.stdout ?? ''
}

export function parseArgs(argv) {
  const opts = { release: true, target: undefined }
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]
    if (a === '--debug' || a === '-d') opts.release = false
    else if (a === '--release' || a === '-r') opts.release = true
    else if (a === '--target' || a === '-t') opts.target = argv[++i]
    else if (a.startsWith('--target=')) opts.target = a.slice('--target='.length)
    else fail(`unknown argument ${a}`)
  }
  if (opts.target === '') fail('--target needs a value')
  return opts
}

function hostTriple() {
  const out = run('rustc', ['-vV'])
  const line = out.split('\n').find((l) => l.startsWith('host: '))
  if (!line) fail('rustc -vV printed no host triple')
  return line.slice('host: '.length).trim()
}

function targetDir() {
  const meta = JSON.parse(run('cargo', ['metadata', '--format-version', '1', '--no-deps', '--offline']))
  return meta.target_directory
}

const exe = (triple) => (triple.includes('windows') ? '.exe' : '')

/** Builds the relay for one real triple; returns the built file. */
function cargoBuild(triple, explicitTarget, release) {
  const args = ['build', '-p', 'mode-hook']
  if (release) args.push('--release')
  if (explicitTarget) args.push('--target', triple)
  if (process.env.CI) args.push('--locked')
  console.log(`build-hook: cargo ${args.join(' ')}`)
  run('cargo', args, { stdio: 'inherit' })
  const profileDir = release ? 'release' : 'debug'
  const base = explicitTarget ? join(targetDir(), triple, profileDir) : join(targetDir(), profileDir)
  return join(base, `mode-hook${exe(triple)}`)
}

export function sidecarPath(triple) {
  return join(outDir, `mode-hook-${triple}${exe(triple)}`)
}

/** Builds the relay and copies it to binaries/; returns the sidecar path. */
export function buildHook(opts) {
  const triple = opts.target ?? hostTriple()
  mkdirSync(outDir, { recursive: true })
  const dest = sidecarPath(triple)
  if (triple === 'universal-apple-darwin') {
    const parts = ['aarch64-apple-darwin', 'x86_64-apple-darwin'].map((t) => cargoBuild(t, true, opts.release))
    run('lipo', ['-create', '-output', dest, ...parts], { stdio: 'inherit' })
  } else {
    copyFileSync(cargoBuild(triple, opts.target !== undefined, opts.release), dest)
  }
  if (!triple.includes('windows')) chmodSync(dest, 0o755)
  console.log(`build-hook: ${opts.release ? 'release' : 'debug'} relay -> ${dest}`)
  return dest
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  buildHook(parseArgs(process.argv.slice(2)))
}
