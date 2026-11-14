// The Agents view (03 §7E) and its routes (03 §2.5): `agents` the list,
// `agents/<id>` the edit page (`new` for a new one), `agents/<id>/<tab>` one
// of its tabs (identity · brain · knows · does · rules · advanced).
import { useEffect } from 'react'
import { useAgents } from '../../state/agents-store'
import { AgentsList, DeleteHost } from './AgentsList'
import { AGENT_TABS, Editor, type AgentTab } from './Editor'
import './agents.css'

export function AgentsView({ rest }: { rest?: string }) {
  const [id, sub] = (rest ?? '').split('/') as [string | undefined, string | undefined]
  const tab = AGENT_TABS.find(([t]) => t === sub)?.[0] ?? 'identity'
  useEffect(() => {
    if (!useAgents.getState().loaded) void useAgents.getState().load()
  }, [])
  return (
    <>
      {!id ? <AgentsList /> : <Editor key={id} id={id} tab={tab as AgentTab} />}
      <DeleteHost />
    </>
  )
}
