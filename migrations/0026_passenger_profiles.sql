-- Saved passenger profiles are intentionally separate from booking snapshots.
-- Every value is entered by the traveller/profile owner. Checkout defaults,
-- account or supplier contacts, and values merely prefilled from a profile are
-- never written back unless the user edits that field and chooses to save it.

create table if not exists public.passenger_profile_ref_counters (
  ref_date date primary key,
  last_value integer not null default 0 check (last_value between 0 and 999999),
  updated_at timestamptz not null default now()
);


create or replace function public.allocate_passenger_profile_ref_for(p_date date)
returns text
language plpgsql
as $$
declare
  v_next integer;
begin
  insert into public.passenger_profile_ref_counters as c (ref_date, last_value)
  values (p_date, 1)
  on conflict (ref_date) do update
    set last_value = c.last_value + 1,
        updated_at = now()
  returning c.last_value into v_next;

  return 'STP' || to_char(p_date, 'YYMMDD') || lpad(v_next::text, 6, '0');
end;
$$;

create or replace function public.allocate_passenger_profile_ref()
returns text
language sql
as $$
  select public.allocate_passenger_profile_ref_for(
    (now() at time zone 'Asia/Dhaka')::date
  );
$$;


create table if not exists public.passenger_profiles (
  id uuid primary key default gen_random_uuid(),
  public_ref text not null default public.allocate_passenger_profile_ref(),
  owner_user_id text not null check (owner_user_id ~ '^user_[A-Za-z0-9_]+$' and char_length(owner_user_id) <= 128),

  passenger_type text not null check (passenger_type in ('ADT', 'CHD', 'CNN', 'INF', 'INS')),
  title text not null check (title in ('Mr', 'Mrs', 'Ms', 'Mstr', 'Miss')),
  given_name text not null check (char_length(given_name) between 1 and 60),
  surname text not null check (char_length(surname) between 1 and 60),
  gender text not null check (gender in ('Male', 'Female')),
  nationality text not null check (nationality ~ '^[A-Z]{2}$'),

  -- Optional contact values are saved only when the user types them. These
  -- are never seeded from supplier/company configuration or an account profile.
  phone_country_code text check (phone_country_code is null or phone_country_code ~ '^\+[0-9]{1,4}$'),
  phone text check (phone is null or phone ~ '^[0-9]{6,15}$'),
  email text check (email is null or char_length(email) <= 254),

  -- Optional document and preference fields. They are profile data only and
  -- must never be seeded from a supplier, account, or booking default.
  date_of_birth date,
  passport_number text check (passport_number is null or passport_number ~ '^[A-Z0-9]{5,20}$'),
  passport_expiry date,
  issuing_country text check (issuing_country is null or issuing_country ~ '^[A-Z]{2}$'),
  loyalty_airline_code text check (loyalty_airline_code is null or loyalty_airline_code ~ '^[A-Z0-9]{2,10}$'),
  loyalty_account_number text check (loyalty_account_number is null or char_length(loyalty_account_number) <= 50),
  ssr_requests jsonb not null default '[]'::jsonb
    check (jsonb_typeof(ssr_requests) = 'array'),
  organization text check (organization is null or char_length(organization) <= 120),

  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),

  constraint passenger_profiles_public_ref_format_check
    check (public_ref ~ '^STP[0-9]{12}$'),
  constraint passenger_profiles_title_match_check
    check (
      (passenger_type = 'ADT' and gender = 'Male' and title = 'Mr')
      or (passenger_type = 'ADT' and gender = 'Female' and title in ('Ms', 'Mrs'))
      or (passenger_type in ('CHD', 'CNN', 'INF', 'INS') and gender = 'Male' and title = 'Mstr')
      or (passenger_type in ('CHD', 'CNN', 'INF', 'INS') and gender = 'Female' and title = 'Miss')
    ),
  constraint passenger_profiles_phone_pair_check
    check (
      (phone is null and phone_country_code is null)
      or (phone is not null and phone_country_code is not null)
    )
);

create unique index if not exists passenger_profiles_public_ref_key
  on public.passenger_profiles (public_ref);
create index if not exists passenger_profiles_owner_created_idx
  on public.passenger_profiles (owner_user_id, created_at desc);
create index if not exists passenger_profiles_name_idx
  on public.passenger_profiles (owner_user_id, surname, given_name);

create function public.touch_passenger_profile_updated_at()
returns trigger language plpgsql as $$
begin new.updated_at = now(); return new; end;
$$;

drop trigger if exists passenger_profiles_touch_updated_at on public.passenger_profiles;
create trigger passenger_profiles_touch_updated_at
  before update on public.passenger_profiles
  for each row execute function public.touch_passenger_profile_updated_at();

create or replace function public.prevent_passenger_profile_ref_change()
returns trigger
language plpgsql
as $$
begin
  if new.public_ref is distinct from old.public_ref or new.owner_user_id is distinct from old.owner_user_id then
    raise exception 'Passenger profile ownership and reference are immutable';
  end if;
  return new;
end;
$$;

drop trigger if exists passenger_profiles_public_ref_immutable on public.passenger_profiles;
create trigger passenger_profiles_public_ref_immutable
  before update on public.passenger_profiles
  for each row execute function public.prevent_passenger_profile_ref_change();



comment on table public.passenger_profiles is
  'User-saved passenger profiles. Values are retained only after explicit user entry; prefilled checkout, account, and supplier data is not saved.';
comment on column public.passenger_profiles.public_ref is
  'Immutable public passenger identifier, STPYYMMDD###### (for example, STP260822000001).';

-- Preserve the existing frontend response contract; never expose owner IDs.
CREATE FUNCTION portal_passenger_json(p passenger_profiles) RETURNS jsonb LANGUAGE sql IMMUTABLE AS $$
 SELECT jsonb_build_object(
  'id', to_jsonb(p.id),
  'publicRef', to_jsonb(p.public_ref),
  'passengerType', COALESCE(to_jsonb(p.passenger_type), '""'::jsonb),
  'title', COALESCE(to_jsonb(p.title), '""'::jsonb),
  'firstName', COALESCE(to_jsonb(p.given_name), '""'::jsonb),
  'lastName', COALESCE(to_jsonb(p.surname), '""'::jsonb),
  'gender', COALESCE(to_jsonb(p.gender), '""'::jsonb),
  'nationality', COALESCE(to_jsonb(p.nationality), '""'::jsonb),
  'phoneCountryCode', COALESCE(to_jsonb(p.phone_country_code), '""'::jsonb),
  'phone', COALESCE(to_jsonb(p.phone), '""'::jsonb),
  'email', COALESCE(to_jsonb(p.email), '""'::jsonb),
  'dateOfBirth', COALESCE(to_jsonb(p.date_of_birth), '""'::jsonb),
  'passportNumber', COALESCE(to_jsonb(p.passport_number), '""'::jsonb),
  'passportExpiry', COALESCE(to_jsonb(p.passport_expiry), '""'::jsonb),
  'issuingCountry', COALESCE(to_jsonb(p.issuing_country), '""'::jsonb),
  'loyaltyAirlineCode', COALESCE(to_jsonb(p.loyalty_airline_code), '""'::jsonb),
  'loyaltyAccountNumber', COALESCE(to_jsonb(p.loyalty_account_number), '""'::jsonb),
  'ssrRequests', to_jsonb(p.ssr_requests),
  'organization', COALESCE(to_jsonb(p.organization), '""'::jsonb),
  'createdAt', to_jsonb(p.created_at),
  'updatedAt', to_jsonb(p.updated_at)
 );
$$;

-- Access is only through the trusted Rust service, not an anonymous SQL role.
REVOKE ALL ON passenger_profiles, passenger_profile_ref_counters FROM PUBLIC;

