-- Rename only product defaults; preserve customer names and service credentials.
UPDATE notification_settings SET from_name = 'QonPanel'
WHERE from_name IN ('Qpanel', 'Nusa Panel', 'Vpanel');
UPDATE notification_settings SET from_name = 'QonPanel Alerts'
WHERE from_name IN ('Qpanel Alerts', 'Nusa Panel Alerts', 'Vpanel Alerts');
UPDATE settings SET value = 'QonPanel'
WHERE key IN ('panel.title', 'site.title') AND value IN ('Qpanel', 'Nusa Panel', 'Vpanel');
