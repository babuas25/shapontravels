-- Subject changes carry administrative authority.  The regular application
-- role must never be able to invoke the one-time break-glass remap function,
-- even if it can reach the database through a future application defect.
-- The controlled production run executes it as the local PostgreSQL
-- administrator after the reviewed preflight and backup/restore proof.
REVOKE ALL ON FUNCTION portal_identity_apply_subject_remap(UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT) FROM PUBLIC;
