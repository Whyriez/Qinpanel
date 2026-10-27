-- Update only the former product defaults; preserve configured customer branding.
UPDATE notification_settings SET from_name = 'Qpanel'
WHERE from_name = 'Nusa Panel';

UPDATE notification_settings SET from_name = 'Qpanel Alerts'
WHERE from_name = 'Nusa Panel Alerts';

UPDATE settings SET value = 'Qpanel'
WHERE key IN ('panel.title', 'site.title') AND value = 'Nusa Panel';
