-- 0007 — What an agent works on and the rules it keeps (03 §7E.2 Does and
-- Rules): the repositories a launch starts in and adds, the MCP servers it
-- may use (names of the ones Claude Code already has configured), short
-- free-text rules added to its system prompt, and whether its memories are.

ALTER TABLE agent_profiles ADD COLUMN repos_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(repos_json));            -- folders; the first is where a launch with no folder starts, the rest --add-dir
ALTER TABLE agent_profiles ADD COLUMN connectors_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(connectors_json));  -- MCP server names; [] = the CLI's own MCP config, else --mcp-config <file> --strict-mcp-config
ALTER TABLE agent_profiles ADD COLUMN rules_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(rules_json));            -- short instructions, a "Rules" section of the system prompt
ALTER TABLE agent_profiles ADD COLUMN use_memories INTEGER NOT NULL DEFAULT 1;                                          -- 0 leaves the memories out of the system prompt
