// `ui/src/notes` (package 7, web half): what the shell and other views use.
// Routing to the view and the global ⌘/Ctrl+K binding are the shell's.
export { NotesView, useNotesStart, type NotesViewProps } from './NotesView'
export { QueryBar, FloatingQuery, PLACEHOLDERS, type QueryBarProps, type FloatingQueryProps } from './QueryBar'
export { QueryCard, CardLayer, type QueryCardProps, type CardLayerProps } from './QueryCard'
export { cardTitle, chipText, typeChip } from './title'
export { planDrop, addTaskPlan, rowGlyph, nextStatus } from './rules'
/** Browser-only scaffolding: registers the notes commands on `window.__mode_mock` (call outside Tauri). */
export { installNotesMock, NotesVault, MOCK_TODAY } from './mock'
export { useNotesStore } from '../state/notes-store'
