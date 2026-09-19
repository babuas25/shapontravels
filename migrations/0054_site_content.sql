CREATE TABLE site_content (
    kind text PRIMARY KEY CHECK (kind IN ('announcements','offers','promotion','logo','background')),
    data jsonb NOT NULL,
    version bigint NOT NULL DEFAULT 1 CHECK(version > 0),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_by uuid REFERENCES portal_users(id)
);
INSERT INTO site_content(kind,data) VALUES
('announcements','{"messages":[],"scrollDurationSeconds":28}'),
('offers','{"heading":"Save Big with Limited-Time Travel Offers","subheading":"Exclusive flight deals, all in one place.","offers":[{"id":"bc665cb7-e276-42b9-a63f-cd931b2c3ea7","eyebrow":"Travel offers","title":"Your first travel offer","description":"Add the details of your travel offer.","active":false,"image":null},{"id":"460b0fb1-dc51-45d0-8473-8c233b9dfb73","eyebrow":"Travel offers","title":"Your second travel offer","description":"Add the details of your travel offer.","active":false,"image":null},{"id":"523f37e1-50d1-4094-8bfe-719af47ff077","eyebrow":"Travel offers","title":"Your third travel offer","description":"Add the details of your travel offer.","active":false,"image":null}]}'),
('promotion','{"enabled":false,"display":{"audience":"b2b","frequencyScope":"tab","pages":["/dashboard","/dashboard/flight-search"],"showOnLogin":true,"delaySeconds":0,"repeatMinutes":60},"autoCloseSeconds":10,"slides":[]}'),
('logo','null'),('background','null');
CREATE TABLE site_media_assets (
    id uuid PRIMARY KEY,
    metadata jsonb NOT NULL,
    uploaded_by uuid NOT NULL REFERENCES portal_users(id),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
