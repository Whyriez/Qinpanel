-- Nusa Panel Database Migration: 20261003000007_user_shell_policy.sql
-- Add optional shell_policy column to users table for per-tenant override (P6-T6)

ALTER TABLE users ADD COLUMN shell_policy TEXT CHECK(shell_policy IS NULL OR shell_policy IN ('none', 'sftp', 'full'));
