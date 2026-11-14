//! One row struct per table of 09 §5, in DDL column order. Booleans
//! (`CHECK (x IN (0,1))`) are `bool`; nullable columns are `Option`; `REAL`
//! is `f64`; rowid aliases (`notes.seq`, `messages.seq`) are `Option<i64>`
//! so an insert with `None` lets SQLite assign the rowid.

// ---------------------------------------------------------------------------
// 5.1 Notes domain
// ---------------------------------------------------------------------------

table! {
    /// `folders` (S).
    Folder => "folders", key = ["id"] {
        id: String,
        parent_id: Option<String>,
        path: String,
        name: String,
        kind: String,
        position: i64,
        icon: Option<String>,
        color: Option<String>,
        created_at: String,
        updated_at: String,
        rev: i64,
        device_id: String,
        deleted_at: Option<String>,
    }
}
syncable!(Folder);

table! {
    /// `notes` (S; the markdown file is the sync unit).
    Note => "notes", key = ["id"] {
        seq: Option<i64>,
        id: String,
        folder_id: Option<String>,
        path: String,
        title: String,
        status: String,
        urgent: bool,
        when_raw: Option<String>,
        when_date: Option<String>,
        body: String,
        tags_text: String,
        excerpt: String,
        word_count: i64,
        task_total: i64,
        task_done: i64,
        pinned: bool,
        archived_at: Option<String>,
        source_kind: String,
        file_mtime: Option<String>,
        file_size: Option<i64>,
        content_hash: Option<String>,
        frontmatter_dirty: bool,
        indexed_at: Option<String>,
        created_at: String,
        updated_at: String,
        rev: i64,
        device_id: String,
        deleted_at: Option<String>,
    }
}
syncable!(Note);

table! {
    /// `note_revisions` (L, local history).
    NoteRevision => "note_revisions", key = ["id"] {
        id: String,
        note_id: String,
        rev: i64,
        origin: String,
        content: String,
        content_hash: String,
        device_id: String,
        created_at: String,
    }
}

table! {
    /// `tags` (S).
    Tag => "tags", key = ["id"] {
        id: String,
        name: String,
        name_norm: String,
        color: Option<String>,
        created_at: String,
        updated_at: String,
        rev: i64,
        device_id: String,
        deleted_at: Option<String>,
    }
}
syncable!(Tag);

table! {
    /// `note_tags` (derived from frontmatter).
    NoteTag => "note_tags", key = ["note_id", "tag_id"] {
        note_id: String,
        tag_id: String,
        position: i64,
    }
}

table! {
    /// `groupings` (S, saved views).
    Grouping => "groupings", key = ["id"] {
        id: String,
        name: String,
        slug: String,
        kind: String,
        filter_json: String,
        sort_json: String,
        glyph: Option<String>,
        color: Option<String>,
        position: i64,
        show_in_overlay: bool,
        show_in_sidebar: bool,
        created_at: String,
        updated_at: String,
        rev: i64,
        device_id: String,
        deleted_at: Option<String>,
    }
}
syncable!(Grouping);

// ---------------------------------------------------------------------------
// 5.2 Sessions domain
// ---------------------------------------------------------------------------

table! {
    /// `sessions` (L).
    Session => "sessions", key = ["id"] {
        id: String,
        claude_session_id: String,
        parent_session_id: Option<String>,
        agent_id: Option<String>,
        agent_type: Option<String>,
        tracking: String,
        launch_mode: String,
        state: String,
        state_since: String,
        state_reason: Option<String>,
        waiting_kind: Option<String>,
        pending_json: Option<String>,
        display_name: String,
        title: Option<String>,
        avatar_color: String,
        cwd: String,
        repo_root: Option<String>,
        git_branch: Option<String>,
        model: Option<String>,
        permission_mode: Option<String>,
        current_tool: Option<String>,
        current_tool_use_id: Option<String>,
        current_mcp_server: Option<String>,
        last_prompt_preview: Option<String>,
        last_assistant_preview: Option<String>,
        transcript_path: Option<String>,
        terminal_app: Option<String>,
        terminal_ref: Option<String>,
        lines_added: i64,
        lines_removed: i64,
        files_changed: i64,
        num_turns: i64,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_write_tokens: i64,
        context_tokens: Option<i64>,
        cost_usd: Option<f64>,
        started_at: String,
        last_event_at: String,
        ended_at: Option<String>,
        end_reason: Option<String>,
        acknowledged_at: Option<String>,
        dismissed_at: Option<String>,
        created_at: String,
        updated_at: String,
        /// 05 §11.2: the accessory the bot mark wears, `""` for the plain mark (0002).
        avatar_variant: String,
        /// The `claude` process last reported for the row and the boot it belongs to (0004, 05 §9.1).
        pid: Option<i64>,
        boot_id: Option<String>,
        /// The agent profile the session was launched as (0005, 03 §7E).
        profile_id: Option<String>,
    }
}

table! {
    /// `agent_profiles` (L): a reusable launch configuration (03 §7E, ADR-0058).
    AgentProfile => "agent_profiles", key = ["id"] {
        id: String,
        name: String,
        title: Option<String>,
        instructions: Option<String>,
        avatar_variant: String,
        model: Option<String>,
        effort: Option<String>,
        fallback_models_json: String,
        ask_before_switching: bool,
        skills_json: String,
        tables_json: String,
        permission_mode: Option<String>,
        allowed_tools_json: String,
        denied_tools_json: String,
        notify: bool,
        workspace: Option<String>,
        claude_path: Option<String>,
        extra_args_json: String,
        isolate_config: bool,
        position: i64,
        created_at: String,
        updated_at: String,
        /// The provider its sessions run on (0006); `None` = Claude Code's own sign-in.
        provider_id: Option<String>,
        /// Repository folders (0007): the first is where a launch with no folder starts.
        repos_json: String,
        /// MCP server names (0007); `[]` = the CLI's own MCP config.
        connectors_json: String,
        /// Free-text rules (0007), a "Rules" section of the system prompt.
        rules_json: String,
        /// 0 leaves the memories out of the system prompt (0007).
        use_memories: bool,
    }
}

table! {
    /// `ai_providers` (L): an endpoint Mode's features or an agent's sessions call (06 §10.1A, ADR-0059).
    AiProvider => "ai_providers", key = ["id"] {
        id: String,
        kind: String,
        name: String,
        base_url: Option<String>,
        auth: String,
        models_json: String,
        extra_json: String,
        enabled: bool,
        builtin: bool,
        position: i64,
        created_at: String,
        updated_at: String,
    }
}

table! {
    /// `ai_routes` (L): the provider and model of one feature, with its backups.
    AiRoute => "ai_routes", key = ["purpose"] {
        purpose: String,
        provider_id: Option<String>,
        model: Option<String>,
        fallbacks_json: String,
        updated_at: String,
    }
}

table! {
    /// `agent_memories` (L): what an agent is told every time it starts.
    AgentMemory => "agent_memories", key = ["id"] {
        id: String,
        profile_id: String,
        title: String,
        body: String,
        position: i64,
        created_at: String,
        updated_at: String,
    }
}

table! {
    /// `session_events` (L).
    SessionEvent => "session_events", key = ["id"] {
        id: String,
        session_id: String,
        seq: i64,
        source: String,
        event: String,
        tool_name: Option<String>,
        tool_use_id: Option<String>,
        agent_id: Option<String>,
        state_before: Option<String>,
        state_after: Option<String>,
        summary: Option<String>,
        payload_json: Option<String>,
        payload_bytes: i64,
        compacted: bool,
        occurred_at: String,
        received_at: String,
    }
}

table! {
    /// `file_changes` (L).
    FileChangeRow => "file_changes", key = ["id"] {
        id: String,
        session_id: String,
        path: String,
        rel_path: Option<String>,
        change_kind: String,
        old_path: Option<String>,
        lines_added: i64,
        lines_removed: i64,
        edit_count: i64,
        source: String,
        last_tool_use_id: Option<String>,
        first_seen_at: String,
        last_seen_at: String,
    }
}

table! {
    /// `session_connectors` (L, derived from hooks and init events).
    SessionConnector => "session_connectors", key = ["session_id", "kind", "name"] {
        session_id: String,
        kind: String,
        name: String,
        status: String,
        detail_json: Option<String>,
        seen_at: String,
    }
}

// ---------------------------------------------------------------------------
// 5.3 Connectors and inbox domain
// ---------------------------------------------------------------------------

table! {
    /// `accounts` (L, device-bound).
    Account => "accounts", key = ["id"] {
        id: String,
        provider: String,
        external_id: String,
        display_name: String,
        email: Option<String>,
        handle: Option<String>,
        team_id: Option<String>,
        team_name: Option<String>,
        avatar_url: Option<String>,
        services_json: String,
        scopes_json: String,
        status: String,
        status_detail: Option<String>,
        keyring_ref: String,
        cursor_json: String,
        color: Option<String>,
        position: i64,
        connected_at: String,
        last_sync_at: Option<String>,
        last_error_at: Option<String>,
        created_at: String,
        updated_at: String,
    }
}

table! {
    /// `account_sources` (L).
    AccountSource => "account_sources", key = ["id"] {
        id: String,
        account_id: String,
        kind: String,
        external_id: String,
        name: String,
        enabled: bool,
        vip: bool,
        poll_seconds: Option<i64>,
        cursor: Option<String>,
        last_polled_at: Option<String>,
        created_at: String,
        updated_at: String,
    }
}

table! {
    /// `contacts` (S).
    Contact => "contacts", key = ["id"] {
        id: String,
        display_name: String,
        primary_email: Option<String>,
        avatar_url: Option<String>,
        company: Option<String>,
        vip: bool,
        vip_note: Option<String>,
        notes: Option<String>,
        message_count: i64,
        last_seen_at: Option<String>,
        created_at: String,
        updated_at: String,
        rev: i64,
        device_id: String,
        deleted_at: Option<String>,
    }
}
syncable!(Contact);

table! {
    /// `contact_identities` (S).
    ContactIdentity => "contact_identities", key = ["id"] {
        id: String,
        contact_id: String,
        kind: String,
        value: String,
        scope: String,
        is_primary: bool,
        created_at: String,
        updated_at: String,
        rev: i64,
        device_id: String,
        deleted_at: Option<String>,
    }
}
syncable!(ContactIdentity);

table! {
    /// `threads` (L, provider cache).
    Thread => "threads", key = ["id"] {
        id: String,
        account_id: String,
        provider: String,
        external_id: String,
        kind: String,
        title: String,
        channel_id: Option<String>,
        channel_name: Option<String>,
        participants_json: String,
        message_count: i64,
        unread_count: i64,
        last_message_at: Option<String>,
        last_snippet: Option<String>,
        urgency_tier: Option<String>,
        is_archived: bool,
        source_url: Option<String>,
        created_at: String,
        updated_at: String,
    }
}

table! {
    /// `messages` (L, provider cache).
    Message => "messages", key = ["id"] {
        seq: Option<i64>,
        id: String,
        account_id: String,
        thread_id: Option<String>,
        source_id: Option<String>,
        provider: String,
        external_id: String,
        rfc_message_id: Option<String>,
        dedup_key: String,
        kind: String,
        sender_contact_id: Option<String>,
        sender_name: String,
        sender_address: String,
        sender_avatar_url: Option<String>,
        channel_id: Option<String>,
        channel_name: Option<String>,
        subject: Option<String>,
        snippet: String,
        body_text: Option<String>,
        body_truncated: bool,
        body_retained_until: Option<String>,
        has_attachments: bool,
        labels_json: String,
        is_direct: bool,
        is_read: bool,
        read_at: Option<String>,
        read_origin: Option<String>,
        is_archived: bool,
        archived_at: Option<String>,
        urgency_tier: String,
        urgency_why: Option<String>,
        urgency_source: Option<String>,
        urgency_model: Option<String>,
        scored_at: Option<String>,
        source_url: String,
        received_at: String,
        fetched_at: String,
        created_at: String,
        updated_at: String,
    }
}

table! {
    /// `connector_writebacks` (L, durable write-back queue).
    ConnectorWriteback => "connector_writebacks", key = ["id"] {
        id: String,
        account_id: String,
        target_kind: String,
        target_id: String,
        action: String,
        external_id: String,
        attempts: i64,
        next_attempt_at: Option<String>,
        last_error: Option<String>,
        created_at: String,
    }
}

// ---------------------------------------------------------------------------
// 5.4 Calendar domain
// ---------------------------------------------------------------------------

table! {
    /// `calendar_events` (L, provider cache).
    CalendarEvent => "calendar_events", key = ["id"] {
        id: String,
        account_id: String,
        calendar_id: String,
        calendar_name: Option<String>,
        external_id: String,
        recurring_event_id: Option<String>,
        ical_uid: Option<String>,
        title: String,
        description: Option<String>,
        location: Option<String>,
        starts_at: String,
        ends_at: String,
        all_day: bool,
        start_date: Option<String>,
        end_date: Option<String>,
        timezone: Option<String>,
        status: String,
        my_response: Option<String>,
        organizer: Option<String>,
        attendees_json: String,
        join_url: Option<String>,
        conference: Option<String>,
        html_link: Option<String>,
        etag: Option<String>,
        synced_at: String,
        created_at: String,
        updated_at: String,
    }
}

// ---------------------------------------------------------------------------
// 5.4A Automations domain
// ---------------------------------------------------------------------------

table! {
    /// `automations` (S).
    Automation => "automations", key = ["id"] {
        id: String,
        name: String,
        enabled: bool,
        trigger_kind: String,
        schedule_json: String,
        action_kind: String,
        action_json: String,
        notify_on_failure: bool,
        keep_runs: i64,
        last_run_at: Option<String>,
        next_run_at: Option<String>,
        position: i64,
        created_at: String,
        updated_at: String,
        rev: i64,
        device_id: String,
        deleted_at: Option<String>,
    }
}
syncable!(Automation);

table! {
    /// `automation_runs` (L, run log).
    AutomationRun => "automation_runs", key = ["id"] {
        id: String,
        automation_id: String,
        trigger_kind: String,
        started_at: String,
        finished_at: Option<String>,
        state: String,
        result_kind: Option<String>,
        result_id: Option<String>,
        error: Option<String>,
        created_at: String,
    }
}

// ---------------------------------------------------------------------------
// 5.5 Cross-cutting tables
// ---------------------------------------------------------------------------

table! {
    /// `links` (S, polymorphic).
    Link => "links", key = ["id"] {
        id: String,
        from_kind: String,
        from_id: String,
        to_kind: String,
        to_id: String,
        link_type: String,
        label: Option<String>,
        created_by: String,
        created_at: String,
        updated_at: String,
        rev: i64,
        device_id: String,
        deleted_at: Option<String>,
    }
}
syncable!(Link);

table! {
    /// `snoozes` (S, app-local).
    Snooze => "snoozes", key = ["id"] {
        id: String,
        target_kind: String,
        target_id: String,
        until: String,
        preset: Option<String>,
        wake_on_reply: bool,
        fired_reason: Option<String>,
        fired_at: Option<String>,
        cancelled_at: Option<String>,
        created_at: String,
        updated_at: String,
        rev: i64,
        device_id: String,
        deleted_at: Option<String>,
    }
}
syncable!(Snooze);

table! {
    /// `alert_rules` (S).
    AlertRule => "alert_rules", key = ["id"] {
        id: String,
        name: String,
        kind: String,
        enabled: bool,
        trigger_json: String,
        channels_json: String,
        sound: Option<String>,
        respect_dnd: bool,
        quiet_hours: bool,
        cooldown_seconds: i64,
        position: i64,
        last_fired_at: Option<String>,
        created_at: String,
        updated_at: String,
        rev: i64,
        device_id: String,
        deleted_at: Option<String>,
    }
}
syncable!(AlertRule);

table! {
    /// `alert_log` (L, 7 days).
    AlertLog => "alert_log", key = ["id"] {
        id: String,
        rule_id: String,
        event: String,
        item_key: String,
        cooldown_key: Option<String>,
        channels_json: String,
        suppressed_reason: Option<String>,
        fired_at: Option<String>,
        created_at: String,
    }
}

table! {
    /// `settings` (S for `scope = 'user'`; rev/device only, a reset deletes the row).
    Setting => "settings", key = ["key"] {
        key: String,
        value_json: String,
        scope: String,
        updated_at: String,
        rev: i64,
        device_id: String,
    }
}

table! {
    /// `themes` (S for user-made packs only).
    Theme => "themes", key = ["id"] {
        id: String,
        slug: String,
        name: String,
        kind: String,
        appearance: String,
        version: String,
        author: Option<String>,
        tokens_json: String,
        source_path: Option<String>,
        created_at: String,
        updated_at: String,
        rev: i64,
        device_id: String,
        deleted_at: Option<String>,
    }
}
syncable!(Theme);

table! {
    /// `hook_installs` (L).
    HookInstall => "hook_installs", key = ["id"] {
        id: String,
        scope: String,
        settings_path: String,
        relay_path: String,
        relay_version: String,
        events_json: String,
        block_hash: String,
        backup_path: Option<String>,
        status: String,
        status_detail: Option<String>,
        installed_at: String,
        verified_at: Option<String>,
        removed_at: Option<String>,
        created_at: String,
        updated_at: String,
    }
}

table! {
    /// `usage_samples` (L).
    UsageSample => "usage_samples", key = ["id"] {
        id: String,
        sampled_at: String,
        source: String,
        purpose: String,
        session_id: Option<String>,
        model: String,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_write_tokens: i64,
        cost_usd: Option<f64>,
        turn_ref: Option<String>,
        created_at: String,
        cache_write_1h_tokens: i64,
    }
}

table! {
    /// `chat_turns` (L, 90 days).
    ChatTurn => "chat_turns", key = ["id"] {
        id: String,
        conversation_id: String,
        role: String,
        content: String,
        citations_json: String,
        model: Option<String>,
        created_at: String,
    }
}

table! {
    /// `activity_days` (L, roll-up for the Stats day tracker).
    ActivityDay => "activity_days", key = ["day"] {
        day: String,
        sessions_started: i64,
        files_changed: i64,
        tokens: i64,
        rolled_at: String,
        messages: i64,
        agents_started: i64,
        lines_added: i64,
        lines_removed: i64,
        longest_session_s: i64,
        origin: String,
    }
}

// ---------------------------------------------------------------------------
// 5.6 Infrastructure
// ---------------------------------------------------------------------------

table! {
    /// `change_log` (written by triggers; read by the future sync client).
    ChangeLog => "change_log", key = ["seq"] {
        seq: i64,
        table_name: String,
        row_id: String,
        op: String,
        rev: i64,
        device_id: String,
        changed_at: String,
        row_json: Option<String>,
        pushed_at: Option<String>,
    }
}

table! {
    /// `schema_migrations` (the migration ledger).
    SchemaMigration => "schema_migrations", key = ["version"] {
        version: i64,
        name: String,
        kind: String,
        checksum: String,
        app_version: String,
        applied_at: String,
    }
}

/// Every table of 09 §5 with its sync class, for the acceptance checks.
pub const TABLES: &[(&str, char)] = &[
    ("folders", 'S'),
    ("notes", 'S'),
    ("note_revisions", 'L'),
    ("tags", 'S'),
    ("note_tags", 'D'),
    ("groupings", 'S'),
    ("sessions", 'L'),
    ("session_events", 'L'),
    ("file_changes", 'L'),
    ("session_connectors", 'L'),
    ("accounts", 'L'),
    ("account_sources", 'L'),
    ("contacts", 'S'),
    ("contact_identities", 'S'),
    ("threads", 'L'),
    ("messages", 'L'),
    ("connector_writebacks", 'L'),
    ("calendar_events", 'L'),
    ("automations", 'S'),
    ("automation_runs", 'L'),
    ("agent_profiles", 'L'),
    ("agent_memories", 'L'),
    ("ai_providers", 'L'),
    ("ai_routes", 'L'),
    ("links", 'S'),
    ("snoozes", 'S'),
    ("alert_rules", 'S'),
    ("alert_log", 'L'),
    ("settings", 'S'),
    ("themes", 'S'),
    ("hook_installs", 'L'),
    ("usage_samples", 'L'),
    ("chat_turns", 'L'),
    ("activity_days", 'L'),
    ("change_log", 'I'),
    ("schema_migrations", 'I'),
];
