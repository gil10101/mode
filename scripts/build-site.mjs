#!/usr/bin/env node
// Builds the site for hosting: site/ flattened into one folder that stands on its own.
//
//   node scripts/build-site.mjs [out]      (default out: .site at the repository root)
//
// In the repository the pages reach outside their folder: the captures, marks and fonts in assets/
// and the kit stylesheet. A host serves one folder, so this writes the pages and their scripts at the
// folder's root, the kit stylesheet beside them as mode.css, and under assets/ exactly the files the
// pages, the stylesheet and the scripts name. It fails when one of those files is missing.
// vercel.json runs it as the build and serves the folder; docs.html answers at /docs.
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const site = resolve(root, 'site')
const out = resolve(root, process.argv[2] ?? '.site')
// the kit stylesheet: ui/src/kit/mode.css is a byte copy of docs/wireframes/mode.css (check-css-sync)
const kit = resolve(root, 'ui', 'src', 'kit', 'mode.css')

rmSync(out, { recursive: true, force: true })
mkdirSync(out, { recursive: true })

const flatten = (text) => text
  .replace(/\.\.\/(?:docs\/wireframes|ui\/src\/kit)\/mode\.css/g, 'mode.css')
  .replace(/\.\.\/assets\//g, 'assets/')

const texts = []
for (const name of readdirSync(site)) {
  if (!/\.(html|css|js)$/.test(name)) continue
  const text = flatten(readFileSync(resolve(site, name), 'utf8'))
  if (/\.\.\//.test(text.replace(/\/\*[\s\S]*?\*\/|<!--[\s\S]*?-->|\/\/[^\n]*/g, ''))) {
    console.error(`build-site: ${name} still reaches outside the folder after flattening`)
    process.exit(1)
  }
  writeFileSync(resolve(out, name), text)
  texts.push(text)
}
const kitText = readFileSync(kit, 'utf8')
writeFileSync(resolve(out, 'mode.css'), kitText)

const wanted = new Set()
for (const text of texts) for (const m of text.matchAll(/assets\/[A-Za-z0-9_./-]+\.[a-z0-9]+/g)) wanted.add(m[0])
let missing = 0
for (const path of [...wanted].sort()) {
  const from = resolve(root, path)
  if (!existsSync(from)) { console.error(`build-site: ${path} is named by the site but is not in the repository`); missing++; continue }
  mkdirSync(dirname(resolve(out, path)), { recursive: true })
  cpSync(from, resolve(out, path))
}
if (missing) process.exit(1)
console.log(`build-site: ${texts.length} pages and scripts, mode.css and ${wanted.size} assets in ${out}`)
