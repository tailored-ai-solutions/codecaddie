CREATE TABLE connected_sources (
  id uuid PRIMARY KEY,
  provider text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE searchable_documents (
  id uuid PRIMARY KEY,
  source_id uuid NOT NULL REFERENCES connected_sources(id),
  body text NOT NULL
);

