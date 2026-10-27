-- Rename only recognized product defaults. Customer branding and SMTP fields
-- are preserved; no user, runtime, credential, migration or storage ID changes.
UPDATE notification_settings SET from_name = 'Qinpanel'
WHERE from_name IN ('QonPanel', 'Qonpanel', 'Qpanel', 'Nusa Panel', 'Vpanel');
UPDATE notification_settings SET from_name = 'Qinpanel Alerts'
WHERE from_name IN ('QonPanel Alerts', 'Qonpanel Alerts', 'Qpanel Alerts', 'Nusa Panel Alerts', 'Vpanel Alerts');
UPDATE settings SET value = 'Qinpanel'
WHERE key IN ('panel.title', 'site.title')
    AND value IN ('QonPanel', 'Qonpanel', 'Qpanel', 'Nusa Panel', 'Vpanel');
UPDATE settings SET value = json_set(value, '$.panel_name', 'Qinpanel')
WHERE key = 'email.template'
    AND CASE WHEN json_valid(value) THEN json_extract(value, '$.panel_name') END
        IN ('QonPanel', 'Qonpanel', 'Qpanel', 'Nusa Panel', 'Vpanel');
