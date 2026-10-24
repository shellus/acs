export type Mode = "official" | "custom";
export type Agent = "codex" | "claude";
export interface Profile {
  id: string;
  name: string | null;
  official_auth?: string | null;
  account_id?: string | null;
  email?: string | null;
  account_name?: string | null;
  custom_key?: string | null;
  custom_model?: string | null;
  custom_effort?: string | null;
  custom_base_url?: string | null;
  model_providers_toml?: string | null;
  custom_proxy_url?: string | null;
}
export interface Fields {
  provider_name: string;
  base_url: string;
  api_key: string;
  model: string;
  effort: string;
  proxy_url: string;
}
export interface Library {
  official_accounts: Profile[];
  custom_providers: Profile[];
  selected_official: string;
  selected_custom: string;
  official_proxy_url?: string | null;
  official_model?: string | null;
  official_effort?: string | null;
}
export interface AgentDraft {
  agent: Agent;
  mode: Mode;
  library: Library;
  custom_fields: Record<string, Fields>;
}
export interface Draft {
  codex: AgentDraft;
  claude: AgentDraft;
}
export interface View {
  revision: string;
  draft: Draft;
  roots: { acs: string; codex: string; claude: string; claude_json: string; claude_desktop: string };
}
export interface Progress {
  phase: string;
  completed: number;
  total: number;
}
