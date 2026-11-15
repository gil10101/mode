// The site's one script (site/index.html, site/docs.html): the header's scrolled state, the three menus
// (hover with intent, click, keyboard; one open at a time; Escape and focus-out close) and the
// phone sheet behind the menu button; then the step lists of the feature sections, which swap the render, the
// fit of each render to its pane, and the small interactions inside the renders; and the hero's copy button.
// Nothing here is needed to read the page: without it the menus stay closed, the section links in
// the sheet are still in the footer of each panel, the GitHub call is always visible and each
// feature shows its first render at the design size.
(() => {
  const header = document.querySelector('.top')
  if (!header) return
  const menu = header.querySelector('.menu')
  const track = menu.querySelector('.track')
  const hl = track.querySelector('.hl')
  const items = [...track.querySelectorAll('.item')]
  const burger = header.querySelector('.burger')
  const sheet = document.getElementById('sheet')
  let openItem = null
  let openTimer = 0
  let closeTimer = 0

  /* ---- scrolled: the bar gains its ground and hairline ---- */
  const onScroll = () => header.classList.toggle('is-scrolled', window.scrollY > 10)
  addEventListener('scroll', onScroll, { passive: true })
  onScroll()

  /* ---- the highlight that slides under the hovered item ---- */
  const moveHl = (li) => {
    if (!li) { hl.removeAttribute('data-shown'); return }
    const el = li.querySelector('.trigger, a')
    hl.style.setProperty('--hl-x', `${el.offsetLeft}px`)
    hl.style.setProperty('--hl-w', `${el.offsetWidth}px`)
    hl.setAttribute('data-shown', '')
  }

  /* ---- one panel open at a time ---- */
  const setOpen = (li) => {
    if (openItem === li) return
    if (openItem) {
      openItem.querySelector('.trigger').setAttribute('aria-expanded', 'false')
      const p = openItem.querySelector('.panel')
      p.classList.remove('is-open')
      p.inert = true
    }
    openItem = li
    if (li) {
      li.querySelector('.trigger').setAttribute('aria-expanded', 'true')
      const p = li.querySelector('.panel')
      p.classList.add('is-open')
      p.inert = false
    }
    menu.dataset.state = li ? 'open' : 'closed'
  }
  const close = () => { clearTimeout(openTimer); setOpen(null); moveHl(null) }

  items.forEach((li) => {
    const trigger = li.querySelector('.trigger')
    li.addEventListener('pointerenter', (e) => {
      if (e.pointerType === 'touch') return
      clearTimeout(closeTimer)
      moveHl(li)
      clearTimeout(openTimer)
      if (trigger) openTimer = setTimeout(() => setOpen(li), openItem ? 0 : 80)
      else setOpen(null)
    })
    if (!trigger) return
    trigger.addEventListener('click', () => {
      clearTimeout(openTimer)
      if (openItem === li) setOpen(null)
      else { setOpen(li); moveHl(li) }
    })
    trigger.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowDown' && openItem === li) { e.preventDefault(); li.querySelector('.panel a')?.focus() }
    })
  })
  menu.addEventListener('pointerleave', () => { clearTimeout(openTimer); closeTimer = setTimeout(close, 220) })
  menu.addEventListener('pointerenter', () => clearTimeout(closeTimer))
  menu.addEventListener('focusout', (e) => { if (!menu.contains(e.relatedTarget)) close() })
  menu.addEventListener('click', (e) => { if (e.target.closest('.panel a')) close() })
  document.addEventListener('pointerdown', (e) => { if (!menu.contains(e.target)) close() })
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape') return
    if (openItem) { openItem.querySelector('.trigger').focus(); close() }
    if (sheet && !sheet.hidden) toggleSheet(false)
  })

  /* ---- the phone sheet ---- */
  const toggleSheet = (on) => {
    if (!sheet) return
    const open = on ?? sheet.hidden
    sheet.hidden = !open
    burger.setAttribute('aria-expanded', String(open))
    document.documentElement.classList.toggle('has-sheet', open)
  }
  burger?.addEventListener('click', () => toggleSheet())
  sheet?.addEventListener('click', (e) => { if (e.target.closest('a')) toggleSheet(false) })
  matchMedia('(min-width: 721px)').addEventListener('change', (e) => { if (e.matches) toggleSheet(false) })
})()

/* ---- the feature sections: a step (click, a resting pointer, the arrow keys, Home and End) shows its render; no rotation ---- */
document.querySelectorAll('.fsteps').forEach((list) => {
  const tabs = [...list.querySelectorAll('[role="tab"][data-view]')]
  const renders = [...list.closest('.fcard').querySelectorAll('.render[data-view]')]
  let timer = 0
  // one column (tablet and phone): the steps are a row of chips and the chosen step's words sit in one line under them
  const note = document.createElement('p')
  note.className = 'fnote'
  list.after(note)
  list.classList.add('has-note')
  const say = (tab) => {
    const words = tab.querySelector('span > span')
    note.textContent = words ? words.textContent : ''
    note.classList.remove('is-in')
    void note.offsetWidth
    note.classList.add('is-in')
  }
  const select = (tab, focus) => {
    tabs.forEach((t) => {
      const on = t === tab
      t.setAttribute('aria-selected', String(on))
      t.tabIndex = on ? 0 : -1
    })
    renders.forEach((r) => {
      const on = r.dataset.view === tab.dataset.view
      r.classList.toggle('is-on', on)
      r.inert = !on
    })
    say(tab)
    if (focus) tab.focus()
  }
  say(tabs.find((t) => t.getAttribute('aria-selected') === 'true') || tabs[0])
  renders.forEach((r) => { r.inert = !r.classList.contains('is-on') })
  // a resting pointer selects, but only one that moved: opening a step reflows the list under a still pointer,
  // and that must not select the step that slid beneath it
  let x = -1
  let y = -1
  let pending = null
  list.addEventListener('pointermove', (e) => {
    if (e.pointerType === 'touch' || (e.clientX === x && e.clientY === y)) return
    x = e.clientX
    y = e.clientY
    const tab = e.target.closest('[role="tab"]')
    if (tab === pending) return
    clearTimeout(timer)
    pending = tab
    if (tab && tab.getAttribute('aria-selected') !== 'true') timer = setTimeout(() => select(tab), 120)
  })
  list.addEventListener('pointerleave', () => { clearTimeout(timer); pending = null })
  tabs.forEach((tab) => {
    tab.tabIndex = tab.getAttribute('aria-selected') === 'true' ? 0 : -1
    tab.addEventListener('click', () => { clearTimeout(timer); pending = tab; select(tab) })
    tab.addEventListener('keydown', (e) => {
      const i = tabs.indexOf(tab)
      const to = { ArrowDown: i + 1, ArrowRight: i + 1, ArrowUp: i - 1, ArrowLeft: i - 1, Home: 0, End: tabs.length - 1 }[e.key]
      if (to === undefined) return
      e.preventDefault()
      select(tabs[(to + tabs.length) % tabs.length], true)
    })
  })
})

/* ---- the renders: drawn at 720 × 520 and scaled to the pane; one column under 900, where the pane follows the width ---- */
{
  const narrow = matchMedia('(max-width: 900px)')
  const fit = (pane) => {
    const w = pane.clientWidth
    const h = pane.clientHeight
    const k = narrow.matches ? (w - 64) / 720 : Math.min((w - 64) / 720, (h - 64) / 520)
    pane.style.setProperty('--k', Math.max(0.1, Math.min(k, 1)).toFixed(4))
  }
  const panes = [...document.querySelectorAll('.fmedia')]
  const ro = new ResizeObserver((entries) => entries.forEach((e) => fit(e.target)))
  panes.forEach((p) => { fit(p); ro.observe(p) })
  narrow.addEventListener('change', () => panes.forEach(fit))
}

/* ---- inside a render: small, real interactions driven by data attributes, one handler for every view ----
   [data-pick] in a [data-group] selects, and its data-detail shows the group's matching [data-detail-of] panel;
   [data-allow] answers the permission (the card folds, the row and the pill turn to working); a segmented control
   or tab strip with data-seg shows its [data-pane-of] pane; [data-install] installs a pack; a switch switches;
   an editor's task ticks; the notch and the top bar open under the pointer; the capture box saves on Enter */
{
  const scopeOf = (el) => el.closest('[data-seg-scope]') || el.closest('.render')
  const answer = (scope) => {
    scope.querySelectorAll('[data-on-allow]').forEach((el) => {
      const what = el.dataset.onAllow
      if (what === 'fold') el.classList.add('is-folded')
      else if (what === 'state') { el.classList.replace('is-waiting', 'is-working'); el.textContent = 'working' }
      else if (what === 'badge') el.classList.replace('is-waiting', 'is-working')
      else if (what === 'text') el.innerHTML = el.dataset.text
      else if (what === 'hide') el.hidden = true
      else if (what === 'show') el.hidden = false
    })
  }
  const install = (render, btn) => {
    const name = btn.closest('[data-detail-of]')?.dataset.name ?? btn.closest('[data-detail]')?.dataset.detail
    const card = btn.closest('.r-pack') ?? render.querySelector(`.r-pack[data-detail="${name}"]`)
    const panel = name && render.querySelector(`.r-would[data-name="${name}"]`)
    for (const b of [card?.querySelector('[data-install]'), panel?.querySelector('[data-install]')]) {
      if (!b) continue
      b.textContent = 'Installed'
      b.setAttribute('aria-pressed', 'true')
    }
    const foot = card?.querySelector('.r-packfoot')
    if (foot && !foot.querySelector('.m-switch')) {
      foot.insertAdjacentHTML('beforeend', '<button class="m-switch" type="button" role="switch" aria-checked="false" aria-label="On"></button><span class="m-t-meta">off</span>')
    }
  }
  document.addEventListener('click', (e) => {
    const render = e.target.closest('.render')
    if (!render) return
    const allow = e.target.closest('[data-allow]')
    if (allow) answer(render)
    const inst = e.target.closest('[data-install]')
    if (inst) install(render, inst)
    const sw = e.target.closest('.m-switch')
    if (sw) {
      const on = !sw.classList.contains('is-on')
      sw.classList.toggle('is-on', on)
      sw.setAttribute('aria-checked', String(on))
      const word = sw.nextElementSibling
      if (word?.matches('.r-packfoot .m-t-meta')) word.textContent = on ? 'on' : 'off'
      if (sw.hasAttribute('data-localonly')) render.querySelector('.r-plist')?.classList.toggle('is-local-only', on)
    }
    const item = e.target.closest('[data-seg] > [data-show]')
    if (item) {
      const seg = item.parentElement
      const scope = scopeOf(seg)
      ;[...seg.children].forEach((c) => c.classList.toggle('is-active', c === item))
      scope.querySelectorAll(`[data-pane-of="${seg.dataset.seg}"]`).forEach((p) => {
        if (scopeOf(p) === scope) p.hidden = p.dataset.pane !== item.dataset.show
      })
      scope.querySelectorAll(`[data-title-of="${seg.dataset.seg}"]`).forEach((t) => { t.textContent = t.dataset[item.dataset.show] })
    }
    const tick = e.target.closest('.r-editor-body .m-task')
    if (tick) {
      const g = tick.querySelector('.m-task-glyph')
      g.dataset.glyph ??= g.textContent === '✓' ? '○' : g.textContent
      const done = tick.classList.toggle('is-done')
      g.textContent = done ? '✓' : g.dataset.glyph
    }
    const pick = !inst && !sw && e.target.closest('[data-pick]')
    if (pick) {
      const group = pick.closest('[data-group]')
      if (!group) return
      group.querySelectorAll('[data-pick]').forEach((p) => {
        if (p.closest('[data-group]') === group) p.classList.toggle('is-selected', p === pick)
      })
      if (pick.dataset.detail) {
        render.querySelectorAll(`[data-detail-of="${group.dataset.group}"]`).forEach((d) => d.classList.toggle('is-on', d.dataset.name === pick.dataset.detail))
      }
    }
  })
  // the pill step's ladder: a hovered or focused rung puts its case under the housing, leaving returns to the first
  document.querySelectorAll('.render [data-cases]').forEach((view) => {
    const show = (n) => {
      view.classList.toggle('has-case', !!n && n !== '1')
      view.querySelectorAll('[data-case-of]').forEach((c) => c.classList.toggle('is-on', c.dataset.caseOf === n))
    }
    view.querySelectorAll('[data-case]').forEach((rung) => {
      rung.addEventListener('pointerenter', () => show(rung.dataset.case))
      rung.addEventListener('pointerleave', () => show(null))
      rung.addEventListener('focusin', () => show(rung.dataset.case))
      rung.addEventListener('focusout', () => show(null))
    })
  })
  document.querySelectorAll('.render [data-hoveropen]').forEach((el) => {
    const open = (on) => el.classList.toggle('is-open', on)
    el.addEventListener('pointerenter', (e) => { if (e.pointerType !== 'touch') open(true) })
    el.addEventListener('pointerleave', (e) => { if (e.pointerType !== 'touch') open(false) })
    el.addEventListener('focusin', () => open(true))
    el.addEventListener('focusout', (e) => { if (!el.contains(e.relatedTarget)) open(false) })
  })
  document.addEventListener('keydown', (e) => {
    const input = e.target.closest?.('.render .m-capture input')
    if (!input || e.key !== 'Enter' || !input.value.trim()) return
    input.outerHTML = '<div class="m-input"><span class="m-capture-saved">Saved · <a href="#notes">Open</a></span></div>'
  })
}

/* ---- the hero's install line: the copy button writes the command to the clipboard and shows a check for 1.6 s
   (the icons crossfade in site.css); without a clipboard the command is selected for a manual copy ---- */
document.querySelectorAll('[data-copy]').forEach((button) => {
  const field = document.getElementById(button.dataset.copy)
  const live = button.parentElement.querySelector('[aria-live]')
  let timer = 0
  const select = () => {
    const range = document.createRange()
    range.selectNodeContents(field)
    const sel = getSelection()
    sel.removeAllRanges()
    sel.addRange(range)
  }
  button.addEventListener('click', async () => {
    try {
      if (!navigator.clipboard) throw new Error('no clipboard')
      await navigator.clipboard.writeText(field.textContent.trim())
    } catch {
      select()
      return
    }
    clearTimeout(timer)
    button.classList.add('is-done')
    live.textContent = 'Copied'
    timer = setTimeout(() => { button.classList.remove('is-done'); live.textContent = '' }, 1600)
  })
})

/* ---- the documentation page's section list (site/docs.html): the entry of the section being read is marked,
   so the list says where the reader is. Nothing happens on a page without the list. ---- */
{
  const links = [...document.querySelectorAll('.docs-nav a[href^="#"]')]
  const sections = links.map((a) => document.getElementById(a.getAttribute('href').slice(1))).filter(Boolean)
  if (sections.length) {
    const mark = () => {
      // the last section whose top has passed under the header; at the page's end, the last one
      let current = sections[0]
      for (const sec of sections) if (sec.getBoundingClientRect().top <= 120) current = sec
      if (innerHeight + scrollY >= document.documentElement.scrollHeight - 2) current = sections[sections.length - 1]
      for (const a of links) {
        if (a.getAttribute('href') === `#${current.id}`) a.setAttribute('aria-current', 'location')
        else a.removeAttribute('aria-current')
      }
    }
    addEventListener('scroll', mark, { passive: true })
    addEventListener('resize', mark)
    mark()
  }
}
