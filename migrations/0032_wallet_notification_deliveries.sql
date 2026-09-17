-- Event expansion does no provider I/O. Delivery claims are per recipient.
ALTER TABLE wallet_notifications DROP CONSTRAINT wallet_notifications_state_check;
ALTER TABLE wallet_notifications ADD CONSTRAINT wallet_notifications_state_check
 CHECK(state IN ('pending','preparing','prepared','suppressed','sending','sent','failed','unknown'));
ALTER TABLE wallet_notifications ADD COLUMN preparation_hash BYTEA;
ALTER TABLE wallet_notifications ADD COLUMN next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp();
-- A claim made by the old channel-level worker may already have sent something.
UPDATE wallet_notifications SET state='unknown',error_code='LEGACY_DELIVERY_REQUIRES_REVIEW' WHERE state='sending';

CREATE TABLE wallet_notification_deliveries (
 id UUID PRIMARY KEY,
 notification_id UUID NOT NULL REFERENCES wallet_notifications(id),
 recipient_key TEXT NOT NULL CHECK(length(recipient_key) BETWEEN 1 AND 200),
 recipient TEXT CHECK(length(recipient) BETWEEN 1 AND 320),
 audience TEXT NOT NULL CHECK(audience IN ('requester','reviewer','partner','admin','archive')),
 content JSONB NOT NULL CHECK(jsonb_typeof(content)='object'),
 state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','sending','sent','failed','unknown','suppressed')),
 generation INTEGER NOT NULL DEFAULT 0 CHECK(generation>=0),
 attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts>=0),
 generation_attempts INTEGER NOT NULL DEFAULT 0 CHECK(generation_attempts>=0),
 claim_token UUID,
 claimed_at TIMESTAMPTZ,
 completed_at TIMESTAMPTZ,
 next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 error_code TEXT CHECK(length(error_code)<=100),
 provider_message_id TEXT CHECK(length(provider_message_id)<=500),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(notification_id,recipient_key),
 CHECK((state='suppressed' AND recipient IS NULL) OR (state<>'suppressed' AND recipient IS NOT NULL)),
 CHECK(state<>'sending' OR (claim_token IS NOT NULL AND claimed_at IS NOT NULL))
);
CREATE INDEX wallet_delivery_pending ON wallet_notification_deliveries(state,next_attempt_at,created_at);
CREATE TABLE wallet_notification_attempts (
 claim_token UUID PRIMARY KEY,
 delivery_id UUID NOT NULL REFERENCES wallet_notification_deliveries(id),
 attempt INTEGER NOT NULL,
 generation INTEGER NOT NULL,
 started_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 completed_at TIMESTAMPTZ,
 outcome TEXT CHECK(outcome IN ('sent','failed','unknown')),
 error_code TEXT,
 provider_message_id TEXT,
 UNIQUE(delivery_id,attempt),
 CHECK((outcome IS NULL)=(completed_at IS NULL))
);
CREATE TABLE wallet_notification_actions (
 id UUID PRIMARY KEY,
 actor_id TEXT NOT NULL,
 request_hash BYTEA NOT NULL CHECK(octet_length(request_hash)=32),
 result JSONB NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE FUNCTION protect_wallet_delivery() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.id,NEW.notification_id,NEW.recipient_key,NEW.recipient,NEW.audience,NEW.content,NEW.created_at)
 IS DISTINCT FROM (OLD.id,OLD.notification_id,OLD.recipient_key,OLD.recipient,OLD.audience,OLD.content,OLD.created_at)
 THEN RAISE EXCEPTION 'wallet delivery snapshot is immutable'; END IF;
 RETURN NEW;
END;
$$;
CREATE TRIGGER wallet_delivery_immutable BEFORE UPDATE ON wallet_notification_deliveries FOR EACH ROW EXECUTE FUNCTION protect_wallet_delivery();
CREATE FUNCTION protect_wallet_delivery_attempt() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF OLD.outcome IS NOT NULL OR
 (NEW.claim_token,NEW.delivery_id,NEW.attempt,NEW.generation,NEW.started_at)
 IS DISTINCT FROM (OLD.claim_token,OLD.delivery_id,OLD.attempt,OLD.generation,OLD.started_at)
 THEN RAISE EXCEPTION 'wallet delivery attempt is immutable'; END IF;
 RETURN NEW;
END;
$$;
CREATE TRIGGER wallet_attempt_immutable BEFORE UPDATE ON wallet_notification_attempts FOR EACH ROW EXECUTE FUNCTION protect_wallet_delivery_attempt();
CREATE TRIGGER wallet_delivery_no_delete BEFORE DELETE OR TRUNCATE ON wallet_notification_deliveries FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE TRIGGER wallet_attempt_no_delete BEFORE DELETE OR TRUNCATE ON wallet_notification_attempts FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE TRIGGER wallet_notification_action_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON wallet_notification_actions FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE FUNCTION protect_wallet_notification_event() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.id,NEW.request_id,NEW.event,NEW.channel,NEW.event_key,NEW.created_at)
 IS DISTINCT FROM (OLD.id,OLD.request_id,OLD.event,OLD.channel,OLD.event_key,OLD.created_at)
 OR (OLD.preparation_hash IS NOT NULL AND NEW.preparation_hash IS DISTINCT FROM OLD.preparation_hash)
 THEN RAISE EXCEPTION 'wallet notification event is immutable'; END IF;
 RETURN NEW;
END;
$$;
CREATE TRIGGER wallet_notification_event_immutable BEFORE UPDATE ON wallet_notifications FOR EACH ROW EXECUTE FUNCTION protect_wallet_notification_event();
CREATE TRIGGER wallet_notification_event_no_delete BEFORE DELETE OR TRUNCATE ON wallet_notifications FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
