CREATE TABLE users (
    id UUID PRIMARY KEY,
    did TEXT NOT NULL UNIQUE,
    handle TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT users_did_length CHECK (
        octet_length(did) BETWEEN 1 AND 2048
    ),
    CONSTRAINT users_handle_length CHECK (
        handle IS NULL OR octet_length(handle) BETWEEN 1 AND 253
    )
);

CREATE TABLE calendar_connections (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    provider_account_id TEXT NOT NULL,
    calendar_id TEXT NOT NULL,
    credentials_ciphertext BYTEA NOT NULL,
    credentials_key_id TEXT NOT NULL,
    scopes TEXT[] NOT NULL DEFAULT '{}',
    token_expires_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'active',
    last_error_code TEXT,
    last_synced_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT calendar_connections_provider CHECK (
        provider IN ('caldav', 'google', 'microsoft')
    ),
    CONSTRAINT calendar_connections_status CHECK (
        status IN ('active', 'degraded', 'revoked')
    ),
    CONSTRAINT calendar_connections_account_id_length CHECK (
        octet_length(provider_account_id) BETWEEN 1 AND 1024
    ),
    CONSTRAINT calendar_connections_calendar_id_length CHECK (
        octet_length(calendar_id) BETWEEN 1 AND 1024
    ),
    CONSTRAINT calendar_connections_key_id_length CHECK (
        octet_length(credentials_key_id) BETWEEN 1 AND 255
    ),
    CONSTRAINT calendar_connections_user_calendar_unique UNIQUE (
        user_id,
        provider,
        calendar_id
    )
);

CREATE INDEX calendar_connections_user_id_idx
    ON calendar_connections (user_id);

CREATE INDEX calendar_connections_token_expiry_idx
    ON calendar_connections (token_expires_at)
    WHERE status = 'active' AND token_expires_at IS NOT NULL;

CREATE TABLE bookings (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    calendar_connection_id UUID
        REFERENCES calendar_connections(id) ON DELETE SET NULL,
    idempotency_key VARCHAR(128) NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    starts_at TIMESTAMPTZ NOT NULL,
    ends_at TIMESTAMPTZ NOT NULL,
    attendee_name TEXT NOT NULL,
    attendee_email TEXT NOT NULL,
    attendee_time_zone TEXT,
    message TEXT,
    provider_event_id TEXT,
    provider_event_etag TEXT,
    last_error_code TEXT,
    confirmed_at TIMESTAMPTZ,
    cancelled_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT bookings_status CHECK (
        status IN (
            'pending',
            'confirmed',
            'cancel_pending',
            'cancelled',
            'failed'
        )
    ),
    CONSTRAINT bookings_time_range CHECK (ends_at > starts_at),
    CONSTRAINT bookings_idempotency_key_length CHECK (
        octet_length(idempotency_key) BETWEEN 16 AND 128
    ),
    CONSTRAINT bookings_attendee_name_length CHECK (
        octet_length(attendee_name) BETWEEN 1 AND 200
    ),
    CONSTRAINT bookings_attendee_email_length CHECK (
        octet_length(attendee_email) BETWEEN 3 AND 320
    ),
    CONSTRAINT bookings_attendee_time_zone_length CHECK (
        attendee_time_zone IS NULL
        OR octet_length(attendee_time_zone) BETWEEN 1 AND 100
    ),
    CONSTRAINT bookings_message_length CHECK (
        message IS NULL OR octet_length(message) <= 2000
    ),
    CONSTRAINT bookings_user_idempotency_unique UNIQUE (
        user_id,
        idempotency_key
    )
);

CREATE INDEX bookings_user_start_idx
    ON bookings (user_id, starts_at);

CREATE INDEX bookings_active_range_idx
    ON bookings (user_id, starts_at, ends_at)
    WHERE status IN ('pending', 'confirmed');

CREATE INDEX bookings_calendar_connection_idx
    ON bookings (calendar_connection_id)
    WHERE calendar_connection_id IS NOT NULL;

CREATE UNIQUE INDEX bookings_provider_event_unique_idx
    ON bookings (calendar_connection_id, provider_event_id)
    WHERE provider_event_id IS NOT NULL;
