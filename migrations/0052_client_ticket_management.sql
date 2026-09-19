-- Explicit opt-in permissions. Existing clients receive no new grants.
ALTER TABLE api_clients DROP CONSTRAINT api_clients_permissions_check;
ALTER TABLE api_clients ADD CONSTRAINT api_clients_permissions_check CHECK
 (permissions <@ ARRAY['search:read','booking','cancellation','ticketing','wallet:read',
                      'ticket-management:read','ticket-management:write']::TEXT[]);
