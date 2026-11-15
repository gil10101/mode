<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/readme/bot-matrix-dark.gif">
    <img src="assets/readme/bot-matrix.gif" width="120" alt="Mode's bot, drawn in squares, blinking and glancing around">
  </picture>
</p>

<h1 align="center">Mode</h1>

<p align="center"><strong>Mode watches your Claude Code sessions, notes, Gmail, Slack and calendar, and puts the one thing that needs you in the MacBook notch.</strong></p>

<p align="center">
  <img alt="status" src="https://img.shields.io/badge/status-alpha-orange">
  <img alt="platforms" src="https://img.shields.io/badge/platforms-macOS%20%7C%20Windows%20%7C%20Linux-blue">
  <img alt="license" src="https://img.shields.io/badge/license-MIT-lightgrey">
</p>

<p align="center"><a href="https://mode-livid.vercel.app">Website</a> · <a href="https://mode-livid.vercel.app/docs">Documentation</a></p>

<p align="center">
  <img src="assets/readme/demo.gif" width="100%" alt="A session asks for permission in the app window; the pill grows out of the notch and the pointer allows it there; the session finishes, the pointer clicks the reply box, and a dictation fills it while the pill shows listening, then transcribing, then the words typed. With the window stepped back, a "Hey Mode, what's on my list today" request grows the notch into a reply card that streams the answer with its citations; then a track plays with its artwork on the resting notch, and the pointer opens the overlay on the Now playing strip, skips to the next track, and plays a recently played track again from the Music tab">
</p>

---

Mode is a desktop command center for developers who run [Claude Code](https://code.claude.com) and also live in Gmail and Slack. It watches three attention streams and shows only what needs you now:

- **Claude Code sessions.** Every session as a roster card: working, waiting for you, failed, unread, idle, done, untracked. Approve permissions and answer questions from the overlay, send prompts from the app window; open the real terminal when you want it.
- **Notes and tasks.** A markdown vault on disk with folders, statuses (Todo, Doing, Done), an urgent flag, "when" (today, tomorrow, a date) and free tags. Query it in plain language and get floating cards.
- **Inbox and calendar.** Gmail, Slack and Google Calendar, ranked by urgency: important people and direct messages first, then AI scoring with a one-line "why". Triage from the overlay; reply in the source app.

## Install

One command. It fetches the build for this machine from the latest release, checks it against the release's checksum list and installs it (Mode.app into Applications; a .deb or an AppImage on Linux; the installer on Windows), or builds from source when no release fits:

```sh
curl -fsSL https://raw.githubusercontent.com/gil10101/mode/main/scripts/install.sh | sh
```

Mode is in alpha and the builds are unsigned for now: macOS asks once in System Settings, Privacy & Security; Windows shows a SmartScreen prompt. Claude Code's hooks are installed from inside the app, with a diff preview, and removed the same way.

## A tour

Every picture is the app itself, showing sample data.

<table>
  <tr>
    <td width="50%"><img src="assets/readme/inbox.webp" width="100%" alt="Inbox"></td>
    <td width="50%"><img src="assets/readme/notes.webp" width="100%" alt="Notes"></td>
  </tr>
  <tr>
    <td><strong>Inbox.</strong> Gmail and Slack ranked by urgency, with a one-line why and links to the session and the note a thread belongs to.</td>
    <td><strong>Notes.</strong> A markdown vault with statuses, when and tags, grouped as saved views, with the query bar underneath.</td>
  </tr>
  <tr>
    <td><img src="assets/readme/calendar.webp" width="100%" alt="Calendar"></td>
    <td><img src="assets/readme/chat.webp" width="100%" alt="Chat"></td>
  </tr>
  <tr>
    <td><strong>Calendar.</strong> The week with the next event's join link, and the agenda for today and tomorrow.</td>
    <td><strong>Chat.</strong> An assistant over your own notes, sessions and messages, with citations and actions.</td>
  </tr>
  <tr>
    <td><img src="assets/readme/automations.webp" width="100%" alt="Automations"></td>
    <td><img src="assets/readme/profiles.webp" width="100%" alt="An agent profile"></td>
  </tr>
  <tr>
    <td><strong>Automations.</strong> Scheduled runs that start a session or ask Chat on your behalf, with their run history.</td>
    <td><strong>Agents.</strong> Reusable ways of working a session is launched as: model and effort, backup models, instructions and memories, tools and rules.</td>
  </tr>
  <tr>
    <td><img src="assets/readme/themes.webp" width="100%" alt="Theme packs"></td>
    <td><img src="assets/readme/integrations.webp" width="100%" alt="Integrations"></td>
  </tr>
  <tr>
    <td><strong>Themes.</strong> System and Paper built in, packs such as Gruvbox, Catppuccin and Nord, and JSON import.</td>
    <td><strong>Integrations.</strong> Claude Code hooks, Gmail, Slack, Google Calendar, the AI backends and the Marketplace, each with its health.</td>
  </tr>
  <tr>
    <td><img src="assets/readme/marketplace.webp" width="100%" alt="The Marketplace catalog"></td>
    <td><img src="assets/readme/backends.webp" width="100%" alt="AI backends"></td>
  </tr>
  <tr>
    <td><strong>Marketplace.</strong> Packs of agents, automations and themes as plain JSON: a read-only catalog to install from, import with a preview, export from any row.</td>
    <td><strong>AI backends.</strong> Providers such as Anthropic, OpenAI-compatible endpoints, Ollama and LM Studio, a route per feature with fallbacks, and the provider an agent's sessions run on.</td>
  </tr>
</table>

<p align="center">
  <img src="assets/readme/overlay.webp" width="100%" alt="The expanded overlay in the notch: the Sessions tab with a permission card, and tabs for Inbox, Notes, Calendar and Music">
</p>

<table>
  <tr>
    <td width="33%"><img src="assets/readme/nowplaying.webp" width="100%" alt="The expanded overlay in the notch with the Now playing strip under its header: artwork, title, artist, previous, pause and next"></td>
    <td width="33%"><img src="assets/readme/music.webp" width="100%" alt="The expanded overlay in the notch on the Music tab: recently played tracks and albums with their artwork and how long ago each played"></td>
    <td width="33%"><img src="assets/readme/reply.webp" width="100%" alt="The notch grown into a reply card: today's three tasks with citation chips, and Open in Chat, Copy and Dismiss"></td>
  </tr>
  <tr>
    <td><strong>Now playing.</strong> The track your local player is on, under the overlay's header, with its controls, and its artwork on the resting notch while it plays.</td>
    <td><strong>Music.</strong> The tracks and albums Mode saw playing on this machine, newest first; a click plays one again in the player it came from.</td>
    <td><strong>Hey Mode.</strong> Ask out loud while another app is in front and the answer streams into the notch, with chips that open the notes it cites.</td>
  </tr>
</table>

### More of it

<table>
  <tr>
    <td width="50%"><img src="assets/readme/files.webp" width="100%" alt="Files changed in a session"></td>
    <td width="50%"><img src="assets/readme/terminal.webp" width="100%" alt="The embedded terminal"></td>
  </tr>
  <tr>
    <td><strong>Files changed.</strong> Every file a session touched, with the diff beside it.</td>
    <td><strong>Terminal.</strong> The real Claude Code terminal for sessions Mode starts or resumes, in its own tab.</td>
  </tr>
  <tr>
    <td><img src="assets/readme/agents.webp" width="100%" alt="Subagents of a session"></td>
    <td><img src="assets/readme/thread.webp" width="100%" alt="An inbox thread"></td>
  </tr>
  <tr>
    <td><strong>Agents.</strong> The subagents a session spawned, each with its own thread.</td>
    <td><strong>Thread.</strong> A Slack thread read in full, with the session and the note it links to.</td>
  </tr>
  <tr>
    <td><img src="assets/readme/month.webp" width="100%" alt="The calendar month"></td>
    <td><img src="assets/readme/board.webp" width="100%" alt="The notes board"></td>
  </tr>
  <tr>
    <td><strong>Month.</strong> The month at a glance, the same events as the week.</td>
    <td><strong>Board.</strong> Notes as columns by status, Todo to Done.</td>
  </tr>
  <tr>
    <td><img src="assets/readme/appearance.webp" width="100%" alt="Appearance settings with theme packs"></td>
    <td><img src="assets/readme/palette.webp" width="100%" alt="The command palette"></td>
  </tr>
  <tr>
    <td><strong>Appearance.</strong> System, Paper and the packs, light or dark or following the OS.</td>
    <td><strong>Command palette.</strong> Every view, session, note and action a few letters away.</td>
  </tr>
  <tr>
    <td><img src="assets/readme/stats.webp" width="100%" alt="Stats"></td>
    <td><img src="assets/readme/shortcuts.webp" width="100%" alt="Shortcuts"></td>
  </tr>
  <tr>
    <td><strong>Stats.</strong> Spend at list price, the week's metrics, a year of activity like a contribution graph, and the breakdown by model, agent and day.</td>
    <td><strong>Shortcuts.</strong> The one place the keys are listed, and where they are changed.</td>
  </tr>
</table>

<p align="center">
  <img src="assets/readme/placements.webp" width="100%" alt="The overlay as a top-center bar and as a right-edge panel">
</p>

<p align="center"><strong>Other machines.</strong> Without a notch the overlay is a top-center bar or a right-edge panel, chosen in onboarding.</p>

## Requirements

macOS 15 Sequoia, Windows 11, or Linux on x86_64. Claude Code installed for session features. A Google account for Gmail and Calendar, Slack workspaces for Slack, and an Anthropic API key or a local model for the AI features.

## License

[MIT](LICENSE).

The brand marks in `assets/brands/` are Simple Icons paths (CC0 1.0) or drawings of Mode's own; each is its owner's trademark and identifies the app or service Mode connects to. Icons are Lucide (ISC). The typefaces are Geist and Geist Mono (OFL 1.1, `assets/fonts/`).
