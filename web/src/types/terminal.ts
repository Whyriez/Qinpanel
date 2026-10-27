export interface TerminalSession {
  session_id: string;
  username: string;
  pid: number;
  cwd: string;
  cols: number;
  rows: number;
  created_at: number;
  ws_url: string;
  closed: boolean;
}

export interface CreateTerminalSessionPayload {
  cwd?: string;
  cols?: number;
  rows?: number;
  tenant?: string;
}

export interface TerminalCloseResponse {
  session_id: string;
  closed: boolean;
  message: string;
}

export interface TerminalResizeResponse {
  session_id: string;
  cols: number;
  rows: number;
}

export type TerminalConnectionStatus = 'connecting' | 'connected' | 'disconnected' | 'reconnecting';

export interface TerminalTabItem {
  id: string;
  title: string;
  cwd: string;
  session?: TerminalSession;
  status: TerminalConnectionStatus;
}
