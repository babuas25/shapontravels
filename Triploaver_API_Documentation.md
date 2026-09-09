# Flight Booking API

*A B2B JSON-over-HTTPS API for shopping, pricing, booking, ticketing, and cancelling airline inventory.*

| | |
|---|---|
| **Supplier API version** | 2.0 |
| **Supplier PDF date** | 2026-05-07 |
| **Documentation reviewed** | 2026-07-18 |
| **Latest integration package** | `Flight-document-v3` (includes multicity fixtures dated 2026-07-16) |
| **Source collection** | `Online Travel Agency (Common).postman_collection.json` |
| **Audience** | OTA partners, travel-tech integrators |

> **ShoponTravels integration note (reviewed 2026-07-30):** This document describes the supplier
> contract. ShoponTravels applies its own B2C/B2B pricing rules, discounts, gross caps, safe-gross
> LCC service margins, and route precedence after Search or RePrice. See
> [`MARKUP.md`](./MARKUP.md) for that application-owned pricing contract.

## Authors

| Name | Role |
|---|---|
| Md. Sihab Uddin | Senior Technical Project Manager |
| Md. Sohid Uddin | Technical Project Manager |
| Tajbiul Ahmed Turjo | Lead Software Engineer |
| Iftekhar Ahamed Siddiquee | Software Engineer |

---

## Table of Contents

1. [Introduction](#1-introduction)
2. [Authentication](#2-authentication)
3. [Conventions](#3-conventions)
4. [Endpoints](#4-endpoints)
   - [4.1 LogIn](#41-login)
   - [4.2 Search](#42-search)
   - [4.3 FareRules](#43-farerules)
   - [4.4 RePrice](#44-reprice)
   - [4.5 Booking](#45-booking)
   - [4.6 Book Cancel](#46-book-cancel)
   - [4.7 NewTicket](#47-newticket)
   - [4.8 AirTicketingDetails](#48-airticketingdetails)
   - [4.9 PNR](#49-pnr)
5. [Error Handling](#5-error-handling)
6. [Webhooks](#6-webhooks)
7. [SDKs & Client Libraries](#7-sdks--client-libraries)
8. [Changelog](#8-changelog)
9. [Need help?](#9-need-help)

---

## 1. Introduction

The Flight Booking API is a JSON-over-HTTPS B2B integration that lets an Online Travel Agency shop, price, book, ticket, or cancel airline inventory on behalf of travellers.

### What you can do

- 🔍 Search one-way, round-trip, and multicity flights by route, date, passenger mix, and cabin class
- 📜 Inspect fare rules before committing to a price
- 💱 Re-price a selected itinerary to confirm the live fare before booking
- 🧾 Book the seats and produce a PNR
- ❌ Cancel a held booking before ticketing
- 🎫 Issue tickets against a confirmed booking
- 📊 Pull ticketing reports for back-office reconciliation

### The booking flow

Every endpoint is part of one ordered pipeline. Each step's response carries server-issued reference tokens that the next step requires — so steps cannot be tested in isolation:

```
LogIn → Search → (optional FareRules) → RePrice → Booking → NewTicket
                                                       ↘ Book Cancel
                                                       ↘ (direct-ticket: Booking
                                                          issues tickets in one step)
```

> **Important:** The reference tokens (`uniqueTransID`, `itemCodeRef`, `priceCodeRef`, `bookingCodeRef`, `pnr`, etc.) are opaque base64-encoded strings issued by the server. Never construct, decode, or modify them on the client — pass each one through verbatim.

> **Note:** When `bookable` is `false` in both the Search response and the RePrice response, the supplier does not support a hold-then-ticket flow for that itinerary. In that case the Booking call issues tickets directly and returns a NewTicket-style response (with `ticketInfoes[].ticketNumbers[]`) — do not call NewTicket separately. See [§4.5](#45-booking) for details.

### Base URLs

The API is split across two UAT hosts:

| Host | Used for |
|---|---|
| `https://searchapi-uat.triplover.com` | `POST /api/Search` only |
| `https://userapi-uat.triplover.com` | LogIn, FareRules, RePrice, Book, Cancel, NewTicket, B2B Reports |

In Postman these are bound to the collection variables `{{search-base-url}}` and `{{base-url}}` respectively.

> **Collection setup:** The variable values bundled in `Flight-document-v3` are not aligned with the two HTTPS UAT hosts above (`search-base-url` is blank and `base-url` retains a legacy value). Override both variables before running the collection; do not rely on the imported defaults.

---

## 2. Authentication

The API uses Bearer-token authentication. Obtain a token via `POST /api/user/apiLogIn`, then send it on every subsequent request:

```
Authorization: Bearer <Token>
```

### How to obtain credentials

API credentials (email + password) are issued by the API provider during partner onboarding. Passwords are exchanged in a base64-encoded form — pass the encoded value directly to the API; do not decode client-side.

> **Note:** The login token expires 30 minutes after issue and applies uniformly to every endpoint in the pipeline. Pipeline references (`uniqueTransID`, `itemCodeRef`, `pnr`, etc.) are not invalidated by token expiry — after re-login, replay the next call with the same references. Build your client to detect a 401 and transparently re-login.

### Example — Login request

```http
POST https://userapi-uat.triplover.com/api/user/apiLogIn
Content-Type: application/json

{
  "email": "YOUR_TRIPLOVER_EMAIL",
  "password": "YOUR_TRIPLOVER_PASSWORD"
}
```

### Example — Login response

```json
{
  "isSuccess": true,
  "message": "Successful",
  "data": {
    "userName": "Hello World",
    "role": "API",
    "email": "YOUR_TRIPLOVER_EMAIL",
    "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
    "tokenCreationTime": "2026-05-07T09:59:32.1167688Z",
    "tokenExpieryTime": "2026-05-07T10:29:32.1167688Z",
    "refreshToken": "d98207ae-dd19-4082-9dc5-64d1d3ceb28c",
    "refreshTokenCreationTime": "2026-05-07T09:59:32.1167986Z",
    "refreshTokenExpieryTime": "2026-05-07T10:39:32.1167986Z"
  }
}
```

> **Tip:** The response carries an explicit `tokenExpieryTime` (30 minutes after issue) and a longer-lived `refreshToken` valid for ~40 minutes. Track `tokenExpieryTime` instead of computing the deadline yourself.

### Example — Authenticated request header

```
Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...
Content-Type: application/json
```

---

## 3. Conventions

### 3.1 Transport & format

All endpoints are `POST` with `Content-Type: application/json`, except `GET /api/B2BReport/AirTicketingDetails/{uniqueTransID}/{status}`.

All responses are JSON. Successful payloads are wrapped in an envelope; primary data is under `item1`, request/transport metadata is under `item2`.

Currency is requested per call via `defaultCurrency`. Today the live UAT environment is fixed to BDT end-to-end.

### 3.2 Standard response envelope

```json
{
  "item1": { /* domain payload — varies per endpoint */ },
  "item2": {
    "apiRef": 15,                                    // numeric API reference
    "uniqueTransID": "TTL638693564518702241",        // echoes the transaction id
    "isSuccess": true,                                // top-level success flag
    "requestTime": "09-Dec-2024 09:54:39 PM",
    "responseTime": "09-Dec-2024 09:54:41 PM",
    "conversionTime": null,
    "timeTicks": 638693780793461482,
    "message": null,
    "isMessageShow": false,
    "ticketType": null
  }
}
```

> **Tip:** Always check `item2.isSuccess` before reading `item1`. On failure, `item2.message` carries a human-readable reason.

### 3.3 Code conventions

| Concept | Codes |
|---|---|
| Cabin class (int) | 1 Economy · 2 Premium Economy · 3 Business · 4 First · 5 Premium First |
| Passenger type | `ADT` adult (12+) · `CHD` child (5–11) · `CNN` child (2–4) · `INF` infant under 2, no seat · `INS` infant under 2, with seat |
| Carrier code | 2-letter IATA (e.g. `BS` US-Bangla, `BG` Biman Bangladesh, `EK` Emirates) |
| Title | Mr · Mrs · Ms · Mstr (master — male child/infant) |
| Country / nationality | ISO-2 country code (e.g. `BD`, `IN`) |
| Phone country code | `+`-prefixed dialing code (e.g. `+88`, `+91`) |
| Booking status | `"Created"` once a booking is held and ready to ticket |
| Segment group | Supplier-provided journey grouping. The latest multicity fixture uses `0` for the first route and `1` for the second; preserve returned values and do not assume one-based numbering. |

> **Note:** The Search request `childs` field counts every child aged 2 to below 12. The server further splits them by `childrenAges[]` into `chd` (5–11) and `cnn` (2–4) in the response — provide accurate per-child ages because the bucketing affects pricing.

### 3.4 Token chain (what flows between calls)

| Field | First produced by | Consumed by |
|---|---|---|
| Token (Bearer) | LogIn | Every subsequent request (Authorization header) |
| `uniqueTransID` | Search | FareRules · RePrice · Booking · Cancel · NewTicket · AirTicketingDetails · PNR |
| `itemCodeRef` | Search | FareRules · RePrice · Booking · Cancel · NewTicket · PNR |
| `segmentCodeRefs[]` | Search | FareRules · RePrice |
| `priceCodeRef` | RePrice | Booking · Cancel · NewTicket · PNR |
| `pnr` / `bookingRefNumber` | Booking | Cancel · NewTicket · PNR |
| `bookingCodeRef` | Booking | Cancel · NewTicket · PNR |
| `ticketingTimeLimit` | Booking · PNR (`lastTicketTime`) | NewTicket |
| `bookingStatus` | Booking | NewTicket (must be passed through, e.g. `"Created"`) |

---

## 4. Endpoints

### 4.1 LogIn

`POST /api/user/apiLogIn`

Issues a Bearer token used to authenticate every other call.

**Host:** `userapi-uat.triplover.com` · **Auth:** none

#### Request body

| Field | Type | Required | Description |
|---|---|---|---|
| `email` | string | ✓ | API user email issued by the API provider |
| `password` | string | ✓ | Base64-encoded password issued by the API provider |

#### Request example

```json
{
  "email": "YOUR_TRIPLOVER_EMAIL",
  "password": "YOUR_TRIPLOVER_PASSWORD"
}
```

#### Response 200

| Field | Type | Description |
|---|---|---|
| `isSuccess` | bool | Top-level success flag |
| `message` | string | Human-readable status |
| `data.userName` | string | Display name of the API user |
| `data.role` | string | Role assigned to the user (e.g. `"API"`) |
| `data.email` | string | API user email (echoed) |
| `data.token` | string | Bearer token — pass on every subsequent request |
| `data.tokenCreationTime` | string (ISO-8601 UTC) | When the token was issued |
| `data.tokenExpieryTime` | string (ISO-8601 UTC) | When the token expires (30 min after creation) |
| `data.refreshToken` | string (UUID) | Use to mint a new token without re-submitting credentials |
| `data.refreshTokenCreationTime` | string (ISO-8601 UTC) | When the refresh token was issued |
| `data.refreshTokenExpieryTime` | string (ISO-8601 UTC) | When the refresh token expires (~40 min after creation) |

```json
{
  "isSuccess": true,
  "message": "Successful",
  "data": {
    "userName": "Hello World",
    "role": "API",
    "email": "YOUR_TRIPLOVER_EMAIL",
    "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
    "tokenCreationTime": "2026-05-07T09:59:32.1167688Z",
    "tokenExpieryTime": "2026-05-07T10:29:32.1167688Z",
    "refreshToken": "d98207ae-dd19-4082-9dc5-64d1d3ceb28c",
    "refreshTokenCreationTime": "2026-05-07T09:59:32.1167986Z",
    "refreshTokenExpieryTime": "2026-05-07T10:39:32.1167986Z"
  }
}
```

#### cURL

```bash
curl -X POST 'https://userapi-uat.triplover.com/api/user/apiLogIn' \
  -H 'Content-Type: application/json' \
  -d '{
    "email": "YOUR_TRIPLOVER_EMAIL",
    "password": "YOUR_TRIPLOVER_PASSWORD"
  }'
```

#### JavaScript (fetch)

```javascript
const res = await fetch('https://userapi-uat.triplover.com/api/user/apiLogIn', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    email: 'YOUR_TRIPLOVER_EMAIL',
    password: 'YOUR_TRIPLOVER_PASSWORD'
  })
});
const { data: { token } } = await res.json();
```

---

### 4.2 Search

`POST /api/Search`

Returns a list of priced itinerary options. Each option carries the references needed by every later step.

**Host:** `searchapi-uat.triplover.com` · **Auth:** Bearer

#### Request body

| Field | Type | Required | Description |
|---|---|---|---|
| `routes[].origin` | string | ✓ | Origin airport IATA code |
| `routes[].destination` | string | ✓ | Destination airport IATA code |
| `routes[].departureDate` | string (`YYYY-MM-DD`) | ✓ | Departure date for that leg |
| `adults` | int | ✓ | Passengers aged 12+ |
| `childs` | int | ✓ | Passengers aged 2 to <12 (server splits into `chd` / `cnn` based on `childrenAges`) |
| `infants` | int | ✓ | Passengers aged 0 to <2 |
| `cabinClass` | int | ✓ | Cabin class enum (see §3.3) |
| `fareType` | int | | Optional fare-type selector. The supplied multicity fixture sends `1`; the supplier PDF and Postman collection do not define the enum, so confirm other values with Triplover. |
| `preferredCarriers` | string[] | ✓ | Allow-list of IATA carrier codes (empty = no preference) |
| `prohibitedCarriers` | string[] | ✓ | Block-list of IATA carrier codes |
| `childrenAges` | int[] | ✓ | Age of each child in `childs`, e.g. `[5, 6]` |

Each entry in `routes[]` is one requested leg, in travel order:

- **Round-trip:** add a second route whose origin/destination reverse the outbound route.
- **Multicity:** add each onward route in order. There is no separate journey-type field.

#### Request example — one-way

```json
{
  "routes": [
    { "origin": "DAC", "destination": "CXB", "departureDate": "2026-07-04" }
  ],
  "adults": 1,
  "childs": 0,
  "infants": 0,
  "cabinClass": 1,
  "preferredCarriers": ["BS"],
  "prohibitedCarriers": [],
  "childrenAges": []
}
```

#### Request example — round-trip

```json
{
  "routes": [
    { "origin": "DAC", "destination": "CXB", "departureDate": "2026-07-04" },
    { "origin": "CXB", "destination": "DAC", "departureDate": "2026-09-04" }
  ],
  "adults": 1,
  "childs": 0,
  "infants": 0,
  "cabinClass": 1,
  "preferredCarriers": ["BS"],
  "prohibitedCarriers": [],
  "childrenAges": []
}
```

#### Request example — multicity

This example is taken from the latest supplier fixture. The first requested leg is DAC → DXB and the second is DXB → SIN:

```json
{
  "routes": [
    { "origin": "DAC", "destination": "DXB", "departureDate": "2026-10-19" },
    { "origin": "DXB", "destination": "SIN", "departureDate": "2026-11-25" }
  ],
  "adults": 1,
  "fareType": 1,
  "childs": 0,
  "infants": 0,
  "cabinClass": 1,
  "preferredCarriers": [],
  "prohibitedCarriers": [],
  "childrenAges": []
}
```

> **Multicity mapping:** `directions[0]` corresponds to `routes[0]`, `directions[1]` to `routes[1]`, and so on. A direction can contain multiple `segments[]` when that route has a connection.

#### Response 200 — top-level shape

`item1.airSearchResponses[]` is the list of priced itineraries. Each entry has:

| Field | Type | Description |
|---|---|---|
| `uniqueTransID` | string | Per-search transaction id; required by every later call |
| `itemCodeRef` | string | Per-itinerary opaque reference; required by every later call |
| `totalPrice` | number | Base + taxes + fees |
| `totalExtraServicePrice` | number | Sum of extras (meals, baggage) |
| `basePrice` | number | Fare before tax/fees/discount |
| `eqivqlBasePrice` | number | Base price re-stated in caller's `defaultCurrency` |
| `taxes` | number | Government taxes + carrier surcharges |
| `platingCarrier` | string | Marketing carrier IATA code |
| `platingCarrierName` | string | Marketing carrier full name |
| `refundable` | bool | Refundability flag |
| `directions` | direction[][] | One sub-list per requested route, in `routes[]` order |
| `bookingComponents` | object[] | Supplier pricing components. Do not assume a one-to-one mapping with `directions`; the latest two-route multicity fixture returns one component. |
| `passengerFares` | object | Per-pax-type fare breakdown (`adt`, `chd`, `cnn`, `inf`, `ins`). Each populated fare can include `fareType`. |
| `passengerCounts` | object | Per-pax-type counts |
| `bookable` | bool | `true` if the supplier supports the standard hold-then-ticket flow. `false` indicates a direct-ticketing itinerary — see [§4.5](#45-booking) for the special Booking response shape it produces. |
| `hasExtraService` | bool | Whether ancillaries are available |
| `brandedFares` | object[] \| null | Branded-fare options (when applicable) |
| `isVisaRequired` | bool | Supplier visa-requirement flag |
| `rbdChangeAllowed` | bool | Whether the reservation booking designator may change |
| `isCodeShared` | bool | Whether the itinerary includes a codeshare |
| `fareTag` | string \| null | Optional supplier fare label |
| `extraBaggageAllowedPTC` | array | Passenger types eligible for extra baggage |
| `commissionOnTaxes` | array | Tax-commission detail returned by the supplier |
| `avlSrc` | string | Supplier availability-source marker; treat as opaque |

The latest multicity fixture returns `fareType: 1` under `passengerFares.adt`. Because the supplier does not document the enum, clients should preserve the value rather than infer additional meanings.

A **direction** entry contains:

| Field | Type | Description |
|---|---|---|
| `from`, `to` | string | Leg origin/destination IATA |
| `fromAirport`, `toAirport` | string | Full airport names |
| `platingCarrierCode`, `platingCarrierName` | string | Operating carrier |
| `stops` | int | Layover count |
| `segments[]` | object | Per-segment detail (see below) |

A **segment** entry contains:

| Field | Type | Description |
|---|---|---|
| `from`, `to` | string | Segment origin/destination IATA |
| `departure`, `arrival` | string (`YYYY-MM-DD HH:mm:ss`) | Local times |
| `airline` | string | Operating airline name |
| `airlineCode` | string | Operating airline IATA |
| `flightNumber` | string | Flight number |
| `segmentCodeRef` | string | Opaque segment reference; required by RePrice and FareRules |
| `serviceClass` | string | Cabin class letter (`Y`, `J`, `F`, …) |
| `bookingClass` | string | RBD letter |
| `bookingCount` | string | Available seats in the RBD |
| `handBaggage` | string | Cabin baggage allowance |
| `baggage[]` | object | Checked baggage per pax type (`{ units, amount, passengerTypeCode }`) |
| `cabinClass` | string | `"Economy"`, `"Business"`, … |
| `details[]` | object | Terminal, equipment, durations |
| `group` | int | Journey-leg id within itinerary |

#### Response example (truncated)

```json
{
  "item1": {
    "airSearchResponses": [
      {
        "uniqueTransID": "TTL638693564518702241",
        "itemCodeRef": "VFRMNjM4Njkz...U2hhcmVkUEND",
        "totalPrice": 34999,
        "basePrice": 26396,
        "taxes": 9499,
        "platingCarrier": "BG",
        "platingCarrierName": "Biman Bangladesh Airlines",
        "refundable": false,
        "directions": [[
          {
            "from": "DAC", "to": "DXB",
            "fromAirport": "Hazrat Shahjalal International Airport",
            "toAirport": "Dubai International Airport",
            "stops": 0,
            "segments": [{
              "from": "DAC", "to": "DXB",
              "departure": "2024-12-25 20:45:00",
              "arrival": "2024-12-26 00:30:00",
              "airline": "Biman Bangladesh Airlines",
              "airlineCode": "BG",
              "flightNumber": "347",
              "segmentCodeRef": "MHwzfDUw",
              "bookingClass": "K",
              "cabinClass": "Economy",
              "baggage": [{ "units": "kg", "amount": 30, "passengerTypeCode": "ADT" }]
            }]
          }
        ]],
        "passengerCounts": { "adt": 1, "chd": 0, "cnn": 0, "inf": 0, "ins": 0 }
      }
    ]
  },
  "item2": { "isSuccess": true, "uniqueTransID": "TTL638693564518702241" }
}
```

> **Tip:** Filter on `brandedFares == null` to pick a "plain" fare without ancillary upsells. The reference Postman collection's Search test script shows the recommended selection logic.

#### cURL

```bash
curl -X POST 'https://searchapi-uat.triplover.com/api/Search' \
  -H 'Authorization: Bearer <token>' \
  -H 'Content-Type: application/json' \
  -d '{
    "routes": [{ "origin": "DAC", "destination": "CXB", "departureDate": "2026-07-04" }],
    "adults": 1, "childs": 0, "infants": 0,
    "cabinClass": 1,
    "preferredCarriers": ["BS"],
    "prohibitedCarriers": [],
    "childrenAges": []
  }'
```

---

### 4.3 FareRules

`POST /api/FareRules`

Returns the structured fare-rule narrative for a Search result (advance-purchase rules, change/cancellation penalties, baggage allowances, etc.). Optional but recommended before display.

**Host:** `userapi-uat.triplover.com` · **Auth:** Bearer

#### Request body

| Field | Type | Required | Description |
|---|---|---|---|
| `itemCodeRef` | string | ✓ | From Search response |
| `uniqueTransID` | string | ✓ | From Search response |
| `segmentCodeRefs` | string[] | ✓ | Collected from `segments[].segmentCodeRef` across the chosen itinerary's directions |
| `brandedFareRefs` | string | | Optional branded-fare reference (empty string when not applicable) |

#### Request example

```json
{
  "itemCodeRef": "VFRMNjM4Njkz...U2hhcmVkUEND",
  "uniqueTransID": "TTL638693564518702241",
  "segmentCodeRefs": ["MHwzfDUw"],
  "brandedFareRefs": ""
}
```

#### Response 200

`item1.fareRuleDetails[]` is a list of `{ type, fareRuleDetail }` entries. Common `type` values: `Seasons`, `ADV RES/TKTG`, `Min Stay`, `Max Stay`, `StopOvers`, `Combinations`, `Surcharges`, `Travel Restrictions`, `Penalties (Or) Change Fee`, `Accompanied Travel Restrictions`.

```json
{
  "item1": {
    "fareRuleDetails": [
      { "type": "Seasons", "fareRuleDetail": "UNLESS OTHERWISE SPECIFIED..." },
      { "type": "ADV RES/TKTG", "fareRuleDetail": "RESERVATIONS AND TICKETING MUST BE COMPLETED..." },
      { "type": "Penalties (Or) Change Fee", "fareRuleDetail": "CHARGE INR 3000 FOR REISSUE..." }
    ],
    "uniqueTransID": "TTL638693584235673842",
    "itemCodeRef": "VFRMNjM4Njkz...VUFwaUdhbGlsZW8="
  },
  "item2": { "isSuccess": true }
}
```

> **Note:** `fareRuleDetail` is multi-line plain text from the GDS — render it in a monospace block for the user.

---

### 4.4 RePrice

`POST /api/Reprice`

Re-validates the live fare for a chosen itinerary. Always call this between Search and Booking — fares can change between when the inventory was indexed and when the customer commits.

**Host:** `userapi-uat.triplover.com` · **Auth:** Bearer

#### Request body

| Field | Type | Required | Description |
|---|---|---|---|
| `uniqueTransID` | string | ✓ | From Search response |
| `itemCodeRef` | string | ✓ | From Search response |
| `segmentCodeRefs` | string[] | ✓ | All segment refs from the chosen itinerary |
| `taxRedemptions` | string[] | | Tax-redemption codes (typically empty) |
| `commissionOnTaxes` | object[] | | Commission rules (typically empty) |
| `brandedFareRefs` | string | | Branded-fare reference (empty string when not applicable) |

#### Request example

```json
{
  "uniqueTransID": "TTL638693564518702241",
  "itemCodeRef": "VFRMNjM4Njkz...U2hhcmVkUEND",
  "segmentCodeRefs": ["MHwzfDUw"],
  "taxRedemptions": [],
  "commissionOnTaxes": [],
  "brandedFareRefs": ""
}
```

For multicity, flatten `segmentCodeRefs` from every selected direction in route order. In the latest fixture, the first route has two segments and the second has one:

```json
{
  "segmentCodeRefs": [
    "MCMwIzEjMSMwIw==",
    "MCMwIzEjMSMxIw==",
    "MCMxIzIjMSMwIw=="
  ]
}
```

Do not generate these references or reduce them to one per route; pass every returned `segmentCodeRef` through unchanged.

#### Response 200

`item1` carries the refreshed price plus a new `priceCodeRef` for the next step.

| Field | Type | Description |
|---|---|---|
| `isPriceChanged` | bool | `true` if the live fare differs from the Search response |
| `priceCodeRef` | string | Required by Booking, Cancel, NewTicket |
| `itemCodeRef` | string | Refreshed itinerary reference |
| `uniqueTransID` | string | Echoed |
| `currency` | string | e.g. `"BDT"` |
| `totalPrice`, `basePrice`, `taxes` | number | Refreshed pricing |
| `directions[][]` | array | Same shape as Search response |
| `passengerFares` | object | Per-pax fare breakdown |
| `bookable` | bool | Final go/no-go for booking. If `false` (and Search also returned `false`), Booking will issue tickets directly — see §4.5. |

For multicity, `directions` keeps the same route order established by Search. The supplied fixture returns two direction groups and demonstrates that RePrice may return `isPriceChanged: true`; the usual customer re-confirmation rule still applies.

```json
{
  "item1": {
    "isPriceChanged": false,
    "priceCodeRef": "VFRMNjM4Njkz...U2hhcmVkUEND",
    "currency": "BDT",
    "uniqueTransID": "TTL638693564518702241",
    "itemCodeRef": "VFRMNjM4Njkz...U2hhcmVkUEND",
    "totalPrice": 34999,
    "basePrice": 26396,
    "taxes": 9499,
    "platingCarrier": "BG",
    "refundable": true,
    "bookable": true
  },
  "item2": { "isSuccess": true }
}
```

> **Important:** If `isPriceChanged` is `true`, surface the new `totalPrice` to the customer and require explicit re-confirmation before calling Booking.

---

### 4.5 Booking

`POST /api/Book`

Holds the seats and produces a PNR. After this call the booking is in `"Created"` status — it must be ticketed (NewTicket) before the airline's `ticketingTimeLimit` or it will be auto-released.

**Host:** `userapi-uat.triplover.com` · **Auth:** Bearer

#### Request body

| Field | Type | Required | Description |
|---|---|---|---|
| `passengerInfoes[]` | object | ✓ | One entry per passenger (see schema below) |
| `BookingWiseContactInfo` | object | | Conditional booking-level contact container observed as `{}` in the latest multicity fixture; omit unless Triplover instructs you to populate it |
| `agentInfo` | object \| null | | Optional override of the agent record on file |
| `taxRedemptions` | string[] | | Tax-redemption codes (typically empty) |
| `priceCodeRef` | string | ✓ | From RePrice response |
| `uniqueTransID` | string | ✓ | From Search response |
| `itemCodeRef` | string | ✓ | From RePrice response (the refreshed value) |
| `commissionOnTaxes` | object[] | | Commission rules (Optional — by agreement) |

`passengerInfoes[]` schema:

| Field | Type | Required | Description |
|---|---|---|---|
| `nameElement.title` | string | ✓ | Mr / Mrs / Ms / Mstr |
| `nameElement.firstName` | string | ✓ | First (and middle) name as on passport |
| `nameElement.lastName` | string | ✓ | Last name as on passport |
| `gender` | string | ✓ | Male / Female |
| `passengerType` | string | ✓ | ADT / CHD / CNN / INF / INS |
| `dateOfBirth` | string (`YYYY-MM-DD`) | ✓ | DOB |
| `documentInfo.documentNumber` | string | ✓ | Passport number |
| `documentInfo.expireDate` | string (`YYYY-MM-DD`) | ✓ | Passport expiry |
| `documentInfo.issuingCountry` | string | ✓ | ISO-2 |
| `documentInfo.nationality` | string | ✓ | ISO-2 |
| `documentInfo.documentType` | string | | Document type code (empty if passport) |
| `documentInfo.frequentFlyerNumber` | string | | FF number |
| `documentInfo.passportCopy` | string | | Base64 PDF/JPEG (Optional — when supplier requires) |
| `documentInfo.visaCopy` | string | | Base64 PDF/JPEG (Optional — when supplier requires) |
| `documentInfo.postCode` | string | | Postal code |
| `contactInfo.phone` | string | ✓ | Phone number (no country code) |
| `contactInfo.phoneCountryCode` | string | ✓ | e.g. `+88` |
| `contactInfo.email` | string | ✓ | Contact email |
| `contactInfo.countryCode` | string | ✓ | ISO-2 |
| `contactInfo.cityName` | string | | City |
| `isLeadPassenger` | bool | | Marks the lead passenger (Optional — defaults to first entry) |

The latest multicity fixture also contains the following passenger fields. They are not defined in the supplier PDF or the Postman collection's canonical Booking request, so treat them as optional/conditional and confirm their semantics before sending non-empty values:

| Field | Type | Fixture value / safe handling |
|---|---|---|
| `nameElement.middleName` | string | Empty when no middle name |
| `passengerKey` | string | Empty for a new passenger; do not invent a key |
| `isQuickPassenger` | bool | `false` in the fixture |
| `aCMExtraServices` | object[] | Empty when no ancillary selection is requested |
| `meal` | object \| null | `null` when no meal is selected |
| `flightAncillary` | object \| null | `null` when no flight ancillary is selected |
| `passengerInfoes[].passportCopy` | string | The fixture uses a filename, while `documentInfo.passportCopy` is documented as base64 content. Omit this duplicate field unless Triplover explicitly requires it. |

#### Request example

```json
{
  "passengerInfoes": [{
    "nameElement": { "title": "Mr", "firstName": "Iftekhar Ahamed", "lastName": "Siddiquee" },
    "gender": "Male",
    "passengerType": "ADT",
    "dateOfBirth": "2000-05-21",
    "documentInfo": {
      "documentNumber": "ASWDCAEWDF",
      "expireDate": "2030-06-26",
      "documentType": "",
      "issuingCountry": "BD",
      "nationality": "BD",
      "frequentFlyerNumber": "",
      "passportCopy": "",
      "visaCopy": "",
      "postCode": ""
    },
    "contactInfo": {
      "phone": "1612378229",
      "email": "iftekhar.ahamed@technonext.com",
      "phoneCountryCode": "+88",
      "countryCode": "BD",
      "cityName": ""
    }
  }],
  "agentInfo": null,
  "taxRedemptions": [],
  "priceCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
  "uniqueTransID": "FST639137447828401390",
  "itemCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE="
}
```

#### Response 200

`item1` carries the PNR and the references required by NewTicket / Cancel.

| Field | Type | Description |
|---|---|---|
| `pnr` | string | GDS / airline PNR |
| `airlinesPNR[]` | string[] | Airline-side PNR(s) (one per validating carrier) |
| `bookingRefNumber` | string | Mirror of `pnr` for back-compat |
| `bookingStatus` | string | `"Created"` on success |
| `ticketingTimeLimit` | string | Airline-imposed deadline before auto-release (empty when not provided) |
| `bookingCodeRef` | string | Required by NewTicket / Cancel |
| `priceCodeRef` | string | Echoed |
| `itemCodeRef` | string | Echoed |
| `uniqueTransID` | string | Echoed |
| `passengerInfoes[]` | object | Echoed passenger details |
| `flightInfo` | object | `directions[][]`, `bookingComponents`, `passengerFares`, `passengerCounts` (same shape as Search) |
| `warnings[]` | string[] | Non-fatal GDS warnings |
| `message` | string | Empty on success |

```json
{
  "item1": {
    "pnr": "09LPFQ",
    "airlinesPNR": ["09LPFQ"],
    "bookingRefNumber": "09LPFQ",
    "bookingStatus": "Created",
    "ticketingTimeLimit": "",
    "bookingCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
    "priceCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
    "itemCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
    "uniqueTransID": "FST639137447828401390",
    "warnings": [],
    "message": ""
  },
  "item2": { "isSuccess": true }
}
```

> **Important:** Persist `pnr`, `bookingCodeRef`, `priceCodeRef`, `itemCodeRef`, `uniqueTransID`, and `ticketingTimeLimit` server-side as soon as Booking returns. They are required for NewTicket and Book Cancel and cannot be re-derived.

> **Multicity:** Booking uses the same endpoint and reference fields; no multicity flag is added. The latest fixture returns `bookingStatus: "Created"` and preserves both route groups under `flightInfo.directions`.

#### Direct ticketing — when `bookable` is `false`

When the chosen itinerary returns `bookable: false` in both the Search response and the RePrice response, the supplier does not support a hold-then-ticket flow for that fare. In that case the Booking call issues tickets immediately and returns a NewTicket-style payload instead of the standard PNR-and-`Created` payload above:

| Field | Type | Description |
|---|---|---|
| `ticketInfoes[]` | object | One entry per passenger, each with `passengerInfo` and `ticketNumbers[]` |
| `ticketInfoes[].ticketNumbers[]` | string[] | E-ticket numbers issued |
| `pnr`, `bookingRefNumber` | string | Issued PNR |
| `ticketCodeRef` | string | Reconciliation reference |
| `bookingCodeRef`, `priceCodeRef`, `itemCodeRef`, `uniqueTransID` | string | Echoed |
| `flightInfo` | object | Same shape as the standard Booking response |

```json
{
  "item1": {
    "warnings": [],
    "ticketInfoes": [{
      "passengerInfo": {
        "nameElement": { "title": "Mr", "firstName": "Iftekhar Ahamed", "lastName": "Siddiquee" },
        "passengerType": "ADT",
        "gender": "Male"
      },
      "ticketNumbers": ["7792411762343"]
    }],
    "pnr": "09LPFQ",
    "ticketCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
    "bookingCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
    "priceCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
    "uniqueTransID": "FST639137447828401390"
  },
  "item2": { "isSuccess": true }
}
```

> **Important:** In the direct-ticket case do not call NewTicket afterwards — the booking is already fulfilled. Detect the direct-ticket branch by checking whether `item1.ticketInfoes` is present (or whether `item1.bookingStatus` is absent) in the Booking response.

---

### 4.6 Book Cancel

`POST /api/Cancel`

Releases a held booking before tickets are issued. Once NewTicket has been called successfully, use the supplier's refund/void process instead — Cancel is for held PNRs only.

**Host:** `userapi-uat.triplover.com` · **Auth:** Bearer

#### Request body

| Field | Type | Required | Description |
|---|---|---|---|
| `PNR` | string | ✓ | From Booking response |
| `BookingRefNumber` | string | ✓ | Same as PNR |
| `UniqueTransID` | string | ✓ | From Search response |
| `PriceCodeRef` | string | ✓ | From RePrice response |
| `ItemCodeRef` | string | ✓ | From RePrice response |
| `BookingCodeRef` | string | ✓ | From Booking response |

#### Request example

```json
{
  "PNR": "09LPFV",
  "BookingRefNumber": "09LPFV",
  "UniqueTransID": "FST639137457262900574",
  "PriceCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
  "ItemCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
  "BookingCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE="
}
```

#### Response 200

| Field | Type | Description |
|---|---|---|
| `isCancel` | bool | `true` on success |
| `uniqueTransID` | string | Echoed |
| `itemCodeRef`, `priceCodeRef`, `bookingCodeRef` | string | Echoed |
| `baseFare`, `tax`, `fees`, `surcharge`, `amoundPaid`, `amountHeld`, `refundPenalty`, `netRefund` | number \| null | Refund accounting (populated only when partial payment was taken) |

```json
{
  "item1": {
    "isCancel": true,
    "uniqueTransID": "FST639137457262900574",
    "itemCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
    "priceCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
    "bookingCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
    "baseFare": null,
    "tax": null,
    "fees": null,
    "surcharge": null,
    "amoundPaid": null,
    "amountHeld": null,
    "refundPenalty": null,
    "netRefund": null
  },
  "item2": { "isSuccess": true }
}
```

---

### 4.7 NewTicket

`POST /api/ticket/NewTicket`

Issues tickets against a confirmed booking. Must complete before the airline-imposed `ticketingTimeLimit`.

**Host:** `userapi-uat.triplover.com` · **Auth:** Bearer

#### Request body

| Field | Type | Required | Description |
|---|---|---|---|
| `PNR` | string | ✓ | From Booking response |
| `BookingRefNumber` | string | ✓ | Same as PNR |
| `UniqueTransID` | string | ✓ | From Search response |
| `PriceCodeRef` | string | ✓ | From RePrice / Booking response |
| `ItemCodeRef` | string | ✓ | From Booking response |
| `BookingCodeRef` | string | ✓ | From Booking response |
| `PreTicketValidationRef` | string | | Pre-ticket validation reference (Optional — when supplier requires it) |
| `IsPartialPayment` | bool | | `true` if ticketing on a partial-payment hold (Optional — by agreement) |
| `TicketWithNewFare` | bool | | Allow ticketing if the latest fare differs from the held price (Optional — by agreement) |
| `commission` | number | | Commission to apply (Optional — by agreement, default `0`) |

#### Request example

```json
{
  "PNR": "09LPFQ",
  "BookingRefNumber": "09LPFQ",
  "UniqueTransID": "FST639137447828401390",
  "PriceCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
  "ItemCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
  "BookingCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE="
}
```

> **Note:** The optional fields above (`PreTicketValidationRef`, `IsPartialPayment`, `TicketWithNewFare`, `commission`) are accepted by agreement only — omit them unless your contract requires them.

#### Response 200

`item1` carries the issued ticket numbers, plus a `ticketCodeRef` you can keep for reconciliation.

| Field | Type | Description |
|---|---|---|
| `ticketInfoes[]` | object | One entry per passenger |
| `ticketInfoes[].passengerInfo` | object | Echoed pax info |
| `ticketInfoes[].ticketNumbers[]` | string[] | E-ticket numbers issued |
| `pnr` | string | Echoed |
| `ticketCodeRef` | string | Reconciliation reference |
| `bookingCodeRef`, `priceCodeRef`, `itemCodeRef`, `uniqueTransID` | string | Echoed |
| `flightInfo` | object | Same shape as Booking response |
| `warnings[]` | string[] | Non-fatal GDS warnings |

```json
{
  "item1": {
    "warnings": [],
    "ticketInfoes": [{
      "passengerInfo": {
        "nameElement": { "title": "MR", "firstName": "Iftekhar Ahamed", "lastName": "Siddiquee" },
        "passengerType": "ADT",
        "gender": "Male"
      },
      "ticketNumbers": ["7792411762343"]
    }],
    "pnr": "09LPFQ",
    "ticketCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
    "bookingCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
    "priceCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
    "uniqueTransID": "FST639137447828401390"
  },
  "item2": { "isSuccess": true }
}
```

> **Important:** Once NewTicket returns success, the booking is fulfilled and Cancel no longer applies. Use the airline's void window or refund process for any subsequent change.

---

### 4.8 AirTicketingDetails

`GET /api/B2BReport/AirTicketingDetails/{uniqueTransID}/{status}`

Pulls the back-office ticketing record for a transaction. Useful for reconciliation, dashboards, and customer-service lookups.

**Host:** `userapi-uat.triplover.com` · **Auth:** Bearer

#### Path parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `uniqueTransID` | string | ✓ | The transaction id from Search/Booking |
| `status` | string | ✓ | Filter — typically `Confirmed` |

#### Request example

```http
GET https://userapi-uat.triplover.com/api/B2BReport/AirTicketingDetails/TTL638693564518702241/Confirmed
Authorization: Bearer <token>
```

#### cURL

```bash
curl -X GET 'https://userapi-uat.triplover.com/api/B2BReport/AirTicketingDetails/TTL638693564518702241/Confirmed' \
  -H 'Authorization: Bearer <token>'
```

#### Response 200

Returns the back-office ticketing record for the transaction.

| Field | Type | Description |
|---|---|---|
| `ticketInfo.status` | string | Lifecycle status — e.g. `"Issued"`, `"Cancelled"`, `"Refunded"` |
| `ticketInfo.statusFor` | string | Status scope — e.g. `"Ticket"`, `"Booking"` |
| `ticketInfo.ticketType` | string | GDS ticket type code (e.g. `"TAW"`) |
| `ticketInfo.isPaid` | bool | Payment captured |
| `ticketInfo.isCompleted` | bool | Workflow complete |
| `ticketInfo.isReissued` | bool | Reissued from a prior ticket |
| `ticketInfo.bookingDate` | string (ISO-8601) | When the booking was made |
| `ticketInfo.issueDate` | string (ISO-8601) | When the ticket was issued |
| `ticketInfo.ticketingPrice` | number | Total amount ticketed |
| `ticketInfo.markup` | number | Supplier-reported agent markup; separate from ShoponTravels `markup_rules` |
| `ticketInfo.bookingId` | int | Internal booking id |
| `ticketInfo.bookingType` | string | e.g. `"Online"` |
| `ticketInfo.journeyType` | string | `"One Way"` / `"Round Trip"` |
| `ticketInfo.pnr`, `ticketInfo.airlinePNRs` | string | GDS / airline PNR(s) |
| `ticketInfo.uniqueTransID` | string | Echoed |
| `ticketInfo.itemCodeRef` | string | Echoed |
| `ticketInfo.tnxNumber` | string | Internal transaction number |
| `ticketInfo.agentCode`, `ticketInfo.agentName`, `ticketInfo.agentEmail`, `ticketInfo.agentPhone`, `ticketInfo.agentAddress` | string | Issuing agent profile |
| `ticketInfo.leadPaxName`, `ticketInfo.leadPaxEmail` | string | Lead passenger contact |
| `ticketInfo.referenceLog` | string (JSON) | Raw reference payload sent to the supplier |
| `ticketInfo.otaInfo` | object | OTA branding — `name`, `regNo`, `iata`, `address`, `phone`, `email` |
| `passengerInfo[]` | object[] | One entry per passenger with full document, contact, and pricing detail |
| `passengerInfo[].ticketNumbers` | string | Comma-separated ticket numbers |
| `passengerInfo[].basePrice`, `tax`, `ait`, `discount`, `totalPrice` | number | Per-passenger fare breakdown |
| `passengerInfo[].isLeadPax` | bool | Lead-passenger flag |

```json
{
  "ticketInfo": {
    "status": "Issued",
    "statusFor": "Ticket",
    "ticketType": "TAW",
    "isPaid": true,
    "isCompleted": true,
    "bookingDate": "2026-05-07T16:01:01.5666667",
    "issueDate": "2026-05-07T16:01:22.76",
    "ticketingPrice": 6166,
    "markup": 0,
    "bookingId": 26660,
    "bookingType": "Online",
    "journeyType": "One Way",
    "pnr": "09LPFQ",
    "airlinePNRs": "09LPFQ",
    "uniqueTransID": "FST639137447828401390",
    "tnxNumber": "IN0058826050700001",
    "agentCode": "FT00588",
    "agentName": "Hello World",
    "agentEmail": "YOUR_TRIPLOVER_EMAIL",
    "leadPaxName": "Iftekhar Ahamed Siddiquee",
    "leadPaxEmail": "iftekhar.ahamed@technonext.com",
    "otaInfo": {
      "name": "Takeoffbd Ltd",
      "regNo": "42343162",
      "address": "Sharif Plaza (5th Floor), 39 Kemal Ataturk Avenue, Banani, Dhaka-1213, BD.",
      "phone": "09613335566"
    }
  },
  "passengerInfo": [{
    "title": "Mr",
    "first": "Iftekhar Ahamed",
    "last": "Siddiquee",
    "fullName": "MR Iftekhar Ahamed Siddiquee",
    "passengerType": "ADT",
    "gender": "Male",
    "email": "iftekhar.ahamed@technonext.com",
    "phone": "1612378229",
    "documentNumber": "ASWDEWDF",
    "expireDate": "2030-06-26T00:00:00",
    "nationality": "BD",
    "isLeadPax": true,
    "basePrice": 3824,
    "tax": 2325,
    "ait": 17,
    "totalPrice": 6166,
    "ticketNumbers": "7792411762343",
    "pnr": "09LPFQ"
  }]
}
```

> **Note:** Unlike the booking pipeline endpoints, this endpoint returns the report payload at the top level — there is no `item1` / `item2` envelope.

---

### 4.9 PNR

`POST /api/pnr`

Retrieves the live PNR record for a held booking — most importantly, the latest ticketing time limit. Use this between Booking and NewTicket when a customer pauses on the payment screen, or as a periodic refresh on long-lived holds where the supplier may have shortened the deadline.

**Host:** `userapi-uat.triplover.com` · **Auth:** Bearer

#### Request body

The body is the same six-field reference set used by Cancel and NewTicket:

| Field | Type | Required | Description |
|---|---|---|---|
| `PNR` | string | ✓ | From Booking response |
| `BookingRefNumber` | string | ✓ | Same as PNR |
| `UniqueTransID` | string | ✓ | From Search response |
| `PriceCodeRef` | string | ✓ | From RePrice response |
| `ItemCodeRef` | string | ✓ | From Booking response |
| `BookingCodeRef` | string | ✓ | From Booking response |

#### Request example

```json
{
  "PNR": "09LPG1",
  "BookingRefNumber": "09LPG1",
  "UniqueTransID": "FST639137478244638089",
  "PriceCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
  "ItemCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
  "BookingCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE="
}
```

#### Response 200

| Field | Type | Description |
|---|---|---|
| `pnr` | string | GDS / airline PNR |
| `airlinePNRs[]` | string[] | Airline-side PNR(s) |
| `bookingRef` | string | Mirror of `pnr` for back-compat |
| `lastTicketTime` | string (`MM/dd/yyyy HH:mm:ss`) | Latest ticketing time limit — issue tickets before this moment or the booking will auto-release |
| `status` | string | Current PNR status (e.g. `"Booked"`) |
| `remarks` | string | Supplier remarks (e.g. `"Option"`) |
| `fareValidationTime` | string \| null | When the held fare was last revalidated |
| `warnings` | string[] \| null | Non-fatal GDS warnings |
| `priceCodeRef`, `itemCodeRef`, `bookingCodeRef` | string | Echoed |
| `uniqueTransID` | string \| null | Echoed (may be null when the supplier doesn't surface it on this call) |

```json
{
  "item1": {
    "pnr": "09LPG1",
    "airlinePNRs": ["09LPG1"],
    "bookingRef": "09LPG1",
    "lastTicketTime": "05/09/2026 16:06:00",
    "status": "Booked",
    "remarks": "Option",
    "warnings": null,
    "fareValidationTime": null,
    "uniqueTransID": null,
    "priceCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
    "itemCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
    "bookingCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE="
  },
  "item2": {
    "apiRef": 1,
    "uniqueTransID": "FST639137478244638089",
    "isSuccess": true,
    "requestTime": "07-May-2026 04:55:20 PM",
    "responseTime": "07-May-2026 04:55:22 PM",
    "isMessageShow": true
  }
}
```

> **Tip:** `lastTicketTime` supersedes the `ticketingTimeLimit` value returned at Booking — when both are available, treat the latest PNR call as authoritative.

#### cURL

```bash
curl -X POST 'https://userapi-uat.triplover.com/api/pnr' \
  -H 'Authorization: Bearer <token>' \
  -H 'Content-Type: application/json' \
  -d '{
    "PNR": "09LPG1",
    "BookingRefNumber": "09LPG1",
    "UniqueTransID": "FST639137478244638089",
    "PriceCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
    "ItemCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE=",
    "BookingCodeRef": "RlNUNjM5MTM3...VVNCYW5nbGE="
  }'
```

---

## 5. Error Handling

### 5.1 Global error format

Errors are returned with the same envelope as success responses; the failure is signalled by `item2.isSuccess == false`. The human-readable reason is in `item2.message`, and `item1` is typically null or partially populated.

```json
{
  "item1": null,
  "item2": {
    "apiRef": 15,
    "uniqueTransID": "TTL638693564518702241",
    "isSuccess": false,
    "message": "Token has expired",
    "isMessageShow": true,
    "ticketType": null
  }
}
```

### 5.2 Common HTTP status codes

| Status | Meaning | Typical cause |
|---|---|---|
| 200 | OK | Domain-level outcome — check `item2.isSuccess`. Success or business-logic failure |
| 400 | Bad Request | Malformed request. Missing required field, wrong type |
| 401 | Unauthorized | Missing or expired token. Token > 30 min old, header absent |
| 403 | Forbidden | Token valid but caller not entitled. Calling an endpoint outside your role |
| 404 | Not Found | Endpoint or referenced resource not found. Wrong host, wrong PNR, expired Search session |
| 429 | Too Many Requests | Rate limited. Aggressive polling — back off |
| 5xx | Server Error | Upstream supplier or platform issue. Retry with backoff; preserve the `uniqueTransID` |

### 5.3 Common business-logic failures

| Scenario | How to detect | Recommended client behaviour |
|---|---|---|
| Token expired mid-flow | 401 or `item2.message` mentions token | Re-call LogIn, retry the failing request with the same body |
| Search session stale | RePrice / Booking returns `isSuccess: false` with "session expired" | Re-run Search, then retry RePrice |
| Fare changed at RePrice | RePrice `isPriceChanged: true` | Surface the new total to the customer; require re-confirmation |
| Inventory gone at Booking | Booking returns `isSuccess: false`, no PNR | Restart from Search |
| Ticketing time limit elapsed | NewTicket returns failure after deadline | Booking is auto-released — restart from Search |

> **Tip:** Always log `apiRef`, `uniqueTransID`, and `timeTicks` from `item2` alongside the request body. Support engineers can correlate any incident from those three values.

---

## 6. Webhooks

The current API version does not deliver outbound webhooks. State changes (ticketing time-limit expiry, supplier-side cancellations) are surfaced when the client next calls a relevant endpoint (`AirTicketingDetails`, `Booking`, etc.). For real-time notifications, poll `AirTicketingDetails` or contact your account manager about the event-bus roadmap.

---

## 7. SDKs & Client Libraries

No first-party SDKs are published for this version of the API. The Postman collection `Online Travel Agency (Common).postman_collection.json` is the executable reference — import it into Postman or run it via Newman to exercise the full pipeline:

```bash
newman run "Online Travel Agency (Common).postman_collection.json" \
  --env-var "base-url=https://userapi-uat.triplover.com" \
  --env-var "search-base-url=https://searchapi-uat.triplover.com"
```

The collection's test scripts auto-chain the flow by extracting `Token`, `UniqueTransID`, `ItemCodeRef`, `SegmentRefs`, `PriceCodeRef`, `Pnr`, and `BookingCodeRef` between requests — review them as a reference implementation when wiring your own client.

The `Flight-document-v3` package also supplies standalone fixtures under `API_USER_TEST_CASES/ONEWAY`, `API_USER_TEST_CASES/ROUNDTRIP`, and `API_USER_TEST_CASES/MULTICITY`. The multicity JSON files are the newest examples and show the complete Search → RePrice → Booking reference chain for multiple ordered routes.

---

## 8. Changelog

### Documentation refresh — 2026-07-18

Reviewed the complete `Flight-document-v3` supplier package. The supplier PDF still identifies the API as v2.0 (2026-05-07); this is a documentation and integration-fixture refresh, not a newly declared API version.

- Added the supplier's 2026-07-16 multicity Search example (DAC → DXB → SIN).
- Documented route-to-direction ordering and the requirement to pass every segment reference from every route into RePrice.
- Clarified the zero-based `group` values observed in the multicity fixture.
- Corrected `bookingComponents` so clients do not assume one component per route.
- Added `fareType` and additional Search response flags observed in the latest fixtures, with explicit caveats where the supplier does not define enum semantics.
- Documented conditional Booking fields observed only in the multicity fixture and distinguished them from the canonical Postman request.
- Flagged the bundled Postman collection's stale/blank host defaults and documented the required HTTPS UAT overrides.
- Confirmed that multicity uses the existing Search → RePrice → Booking pipeline and introduces no new endpoint or authentication flow.

### v2.0 — 2026-05-07

Second major release of the Flight Booking API.

**New endpoints**

- `POST /api/pnr` — retrieves the latest PNR status and the authoritative `lastTicketTime` for a held booking. See §4.9.

**Pipeline behaviour**

- **Direct ticketing.** When `bookable` is `false` in both the Search and RePrice responses, the Booking call now issues tickets in one step and returns a NewTicket-style payload (`ticketInfoes[]` with `ticketNumbers[]`). NewTicket should not be called for these itineraries. See §1, §4.2, §4.4, and §4.5.
- **Reduced request payloads.** Booking, Book Cancel, and NewTicket no longer require the optional commission / SMS / partial-payment fields by default. They remain accepted by agreement and are documented as Optional in each endpoint's request body table.
  - Booking — `isLeadPassenger`, `aCMExtraServices`, `commissionOnTaxes` are now optional.
  - Book Cancel — `IsSmsSend`, `IsPartialPayment`, `commission` are now optional.
  - NewTicket — `PreTicketValidationRef`, `IsSmsSend`, `IsPartialPayment`, `TicketWithNewFare`, `commission`, `isPartialPayment` are now optional.

**Response detail**

- LogIn response expanded — now documents `userName`, `role`, `email`, `tokenCreationTime`, `tokenExpieryTime`, `refreshToken`, `refreshTokenCreationTime`, `refreshTokenExpieryTime`. Token expiry is now explicit in the response.
- `AirTicketingDetails` response schema documented with full `ticketInfo` + `passengerInfo[]` shape.
- Sample data refreshed to the DAC↔CXB / US-Bangla flow that ships with the updated Postman collection.

**Documentation**

- New consumer-facing collection structure: requests grouped by step (LogIn, Search, RePrice, Booking, Book Cancel, NewTicket, FareRules, AirTicketingDetails, PNR).
- Concrete one-way and round-trip example requests inside each step group.
- Documented split-host topology: Search on the search host, all other endpoints on the user-API host.
- Documented 30-minute Bearer-token lifetime and the recommended re-login-and-retry pattern.
- Documented the standard `item1` / `item2` response envelope.
- Token-chain table extended to cover the new PNR endpoint.
- Brand-neutral wording throughout.

### v1.0 — Initial release

- Established the LogIn → Search → RePrice → Book → NewTicket pipeline.
- Introduced the opaque reference-token chain (`uniqueTransID`, `itemCodeRef`, `priceCodeRef`, `bookingCodeRef`).
- Published the original `MasterWebAPI.postman_collection.json`.

---

## 9. Need help?

📬 **Integration support** — contact your account manager.

---

*Flight Booking API · supplier v2.0 · reviewed 2026-07-18*
