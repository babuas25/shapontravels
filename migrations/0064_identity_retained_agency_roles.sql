-- Retain agency ownership, membership and financial history when an operator
-- assigns a non-agency role. Only B2B owners may have an active agency.
ALTER TABLE portal_agencies DROP CONSTRAINT portal_agencies_owner_role_check;
ALTER TABLE portal_agencies ADD CONSTRAINT portal_agencies_owner_role_check
 CHECK (owner_role <> 'b2b_sub');
ALTER TABLE portal_agencies ADD COLUMN role_resume_status TEXT
 CHECK (role_resume_status IN ('active','suspended'));
ALTER TABLE portal_agencies ADD CONSTRAINT portal_agency_active_owner_role
 CHECK (status <> 'active' OR owner_role = 'b2b');
ALTER TABLE portal_agencies ADD CONSTRAINT portal_agency_role_pause
 CHECK (role_resume_status IS NULL OR (status='suspended' AND owner_role<>'b2b'));
ALTER TABLE portal_agency_memberships DROP CONSTRAINT portal_agency_memberships_check;
ALTER TABLE portal_agency_memberships ADD CONSTRAINT portal_agency_memberships_check
 CHECK ((kind='owner' AND user_role<>'b2b_sub') OR (kind='sub' AND user_role<>'b2b'));
-- The existing deferred foreign keys still require both retained role columns
-- to match the current portal user. Identity keys and wallet links do not move.
