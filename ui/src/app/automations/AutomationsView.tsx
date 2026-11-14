// The Automations view (03 §7B) and its routes (03 §2.5): `automations` the
// list, `automations/<id>` the editor (`new` for a new one),
// `automations/<id>/runs` the run history, `automations/<id>/runs/<run>` one run.
import { useEffect } from 'react'
import { useAutomations } from '../../state/automations-store'
import { AutomationsList, DeleteHost } from './AutomationsList'
import { Editor } from './Editor'
import { RunHistory, RunPage } from './Runs'
import './automations.css'

export function AutomationsView({ rest }: { rest?: string }) {
  const [id, sub, runId] = (rest ?? '').split('/') as [string | undefined, string | undefined, string | undefined]
  useEffect(() => {
    if (!useAutomations.getState().loaded) void useAutomations.getState().load()
  }, [])
  return (
    <>
      {!id ? <AutomationsList /> : sub === 'runs' && runId ? <RunPage id={id} runId={runId} /> : sub === 'runs' ? <RunHistory id={id} /> : <Editor key={id} id={id} />}
      <DeleteHost />
    </>
  )
}
