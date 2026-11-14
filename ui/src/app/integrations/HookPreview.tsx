// The hook install diff preview (03 §7C.3): header (file, backup, relay),
// the unified diff with Mode's additions in the add color, foreign hooks kept
// with the "kept as is — not Mode's" gutter note and removals in the remove
// color, the events with their timeout and async flags, and Apply / Cancel /
// Copy diff. Rendered in a dialog (Integrations, Diagnostics, the
// `integrations/claude-code/install` route) and inline in onboarding step 3.
import { Button } from '../../kit/button'
import { Spinner } from '../../kit/primitives'
import { copyText, Dialog } from '../ui'
import { flowResult, useHooks, type HookFlow } from './hooks-store'
import { diffStats, parseUnifiedDiff, tilde } from './model'

export function HookPreview({ flow, onCancel, onDone, applyLabel, cancelLabel = 'Cancel' }: { flow: HookFlow; onCancel?: () => void; onDone?: () => void; applyLabel?: string; cancelLabel?: string }) {
  const apply = useHooks((s) => s.apply)
  const plan = flow.plan
  const lines = plan ? parseUnifiedDiff(plan.diff, plan.conflicts) : []
  const stats = diffStats(lines)
  const busy = flow.phase === 'applying' || flow.phase === 'verifying' || flow.phase === 'loading'
  const result = flowResult(flow)
  const uninstall = flow.action === 'uninstall'
  return (
    <div className="ig-preview" data-testid="hook-preview" data-phase={flow.phase}>
      {plan ? (
        <>
          <div className="ig-preview-head">
            <div className="m-inline">
              <code>{tilde(plan.target.settings_path)}</code>
              <span className="m-diff">
                <span className="add">{`+${stats.added}`}</span> <span className="del">{`−${stats.removed}`}</span>
              </span>
              {!plan.file_existed && <span className="m-t-meta">new file</span>}
            </div>
            <div className="m-meta">{plan.backup_dir ? `Backup will be written to ${tilde(plan.backup_dir)}` : 'A backup is written before the file changes'}</div>
            <div className="m-meta">
              relay <code>{tilde(plan.target.relay_path)}</code>
              {plan.relay_version ? ` · ${plan.relay_version}` : ''}
            </div>
          </div>
          {plan.changed ? (
            <div className="sd-diff ig-diff" role="region" aria-label="Changes to the settings file">
              <pre>
                {lines.map((l, i) =>
                  l.kind === 'file' ? null : (
                    <div key={i} className={`ln ${l.kind === 'add' ? 'add' : l.kind === 'del' ? 'del' : l.kind === 'hunk' ? 'hunk' : ''}${l.keep ? ' ig-keep' : ''}`}>
                      <span className="no">{l.kind === 'hunk' ? '' : l.kind === 'add' ? `+ ${l.new}` : l.kind === 'del' ? `− ${l.old}` : `${l.new}`}</span>
                      <span className="tx">{l.text}</span>
                      {l.keep && <span className="ig-keep-note">kept as is — not Mode's</span>}
                    </div>
                  ),
                )}
              </pre>
            </div>
          ) : (
            <div className="m-meta ig-same">{uninstall ? 'No Mode hooks in this file; nothing to remove.' : 'The file already has these hooks; Apply re-verifies them.'}</div>
          )}
          {!uninstall && plan.events.length > 0 && (
            <div className="ig-events" aria-label="Events">
              <div className="m-group-label">events</div>
              {plan.events.map((e) => (
                <div key={e.event + (e.matcher ?? '')} className="m-stat-row">
                  <span>
                    <code>{e.event}</code>
                    {e.matcher ? <span className="m-t-meta">{` · matcher “${e.matcher}”`}</span> : null}
                  </span>
                  <span className="m-t-meta">{`${e.timeout} s${e.async ? ' · async' : ''}`}</span>
                </div>
              ))}
            </div>
          )}
          <div className="m-meta ig-scope">Mode edits only ~/.claude/settings.json; never a project's .claude/settings.json or .claude/settings.local.json.</div>
        </>
      ) : flow.phase === 'loading' ? (
        <div className="m-meta">
          <Spinner /> Reading the settings file…
        </div>
      ) : uninstall ? (
        <div className="ap-dialog-note" data-testid="uninstall-confirm">
          Mode removes its entries from ~/.claude/settings.json and keeps every other hook. A backup is written first.
        </div>
      ) : null}
      {flow.error && (
        <div className="ap-dialog-note is-bad" role="alert" data-testid="hook-error">
          {flow.error}
        </div>
      )}
      {result && (
        <div className="ap-dialog-note" role="status" data-testid="hook-result">
          {busy && <Spinner />}
          {` ${result}`}
        </div>
      )}
      <div className="m-card-actions">
        {plan && <Button onClick={() => copyText(plan.diff, 'Diff copied')}>Copy diff</Button>}
        {flow.phase === 'done' ? (
          <Button variant="primary" onClick={onDone}>
            Done
          </Button>
        ) : (
          <>
            {onCancel && (
              <Button onClick={onCancel} disabled={flow.phase === 'applying' || flow.phase === 'verifying'}>
                {cancelLabel}
              </Button>
            )}
            <Button variant="primary" disabled={(!plan && !uninstall) || busy} onClick={() => void apply()}>
              {flow.phase === 'failed' && plan ? 'Try again' : (applyLabel ?? (uninstall ? 'Uninstall hooks' : 'Apply'))}
            </Button>
          </>
        )}
      </div>
    </div>
  )
}

/** The dialog form of the preview; closes on Cancel / Done / Esc. */
export function HookDialog({ onClose }: { onClose: () => void }) {
  const flow = useHooks((s) => s.flow)
  if (!flow) return null
  const title = flow.action === 'uninstall' ? 'Uninstall Claude Code hooks' : 'Install Claude Code hooks'
  return (
    <Dialog title={title} onClose={onClose} testId="hook-dialog">
      <HookPreview flow={flow} onCancel={onClose} onDone={onClose} />
    </Dialog>
  )
}
