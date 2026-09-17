# Reconciled B2B fare breakdown

B2B client responses expose an additive `fareBreakdown` object. Render this object as a unit; its service charge and discount explain the difference between the fare components and accepted payable. This is presentation only. Admin-configured tier shares, markup rules, accepted payable, wallet debit and supplier payloads are unchanged.

## Locations

- Search: `item1.airSearchResponses[].fareBreakdown` for newly created offers.
- RePrice: `item1.fareBreakdown`.
- Pricing (`/api/pricing/offer/{id}`, `/api/pricing/reprice/{id}`, `/api/pricing/booking/{id}`): `fareBreakdown`.
- Batch offer pricing: `{offerId}.fareBreakdown`.
- Book, booking status and lookup by reference: `item1.fareBreakdown`, also copied to `item1.flightInfo.fareBreakdown` when flight information exists.
- NewTicket projection uses the accepted quote breakdown, including historical held quotes.
- Portal Hold reads: `pricing.fareBreakdown` and `quote.fareBreakdown`; saved booking responses are enriched in memory.

Historical pricing/status reads derive the object from saved fare components and accepted pricing. They do not use today's markup/tier settings, update the database or call suppliers. Responses without sufficient accepted B2B pricing omit the object. Staff/B2C flows keep their existing contract.

## Fields and example

All money fields are exact two-decimal strings in `currency`. Root values are journey totals; `passengers` contains per-passenger amounts plus a `count`. Multiply each row by its count when calculating root totals.

For accepted booking STR0A4P2Y0A4P2Y:

```json
{
  "fareBreakdown": {
    "version": 1,
    "currency": "BDT",
    "baseFare": "4224.00",
    "taxes": "1125.00",
    "ait": "15.00",
    "serviceCharge": "46.78",
    "discount": "0.00",
    "tierAdjustment": "0.00",
    "gross": "5410.78",
    "payable": "5410.78",
    "passengers": {
      "adt": {
        "count": 1,
        "baseFare": "4224.00",
        "taxes": "1125.00",
        "ait": "15.00",
        "serviceCharge": "46.78",
        "discount": "0.00",
        "tierAdjustment": "0.00",
        "gross": "5410.78",
        "payable": "5410.78"
      }
    }
  }
}
```

Per passenger:

```text
componentTotal = rounded Base + rounded Taxes + rounded AIT
netAdjustment = accepted Payable - componentTotal
serviceCharge = max(netAdjustment, 0)
discount = max(-netAdjustment, 0)
tierAdjustment = 0.00
Payable = Base + Taxes + AIT + Service Charge - Discount
```

AIT is included exactly once. `discount` is the deduction needed to reconcile the displayed components; it is not the legacy `commission` field. In the ordinary discount branch it includes the AIT offset, because original published gross excludes AIT.

When supplier fare plus markup exceeds original gross, display `fareBreakdown.gross = accepted Payable`; tier adjustment is already folded into service charge/discount. Otherwise, displayed gross stays at its original value and service charge is zero. At zero share, original gross equals payable in either branch. Mixed passenger types apply this rule per passenger, then aggregate.

On the example fare, Admin shares 20%, 80%, 90% and 100% give payables 5362.73, 5403.91, 5410.78 and 5417.64 respectively. At 20%, service charge is zero and discount is 1.27. At 90%, service charge is 46.78 and discount is zero. The share is never hard-coded.

## Compatibility

Existing `totalPrice`, pricing `gross`, signed `commission` and supplier-compatible `discountPrice` retain their prior meanings. Clients adopting this presentation must use `fareBreakdown.gross` for displayed Gross and `fareBreakdown.payable` for the final total, and render service charge/discount from the same object. Do not add old commission or AIT again.

Portal price acceptance allows clients to omit the display-only `fareBreakdown`. All original financial and pricing-context fields must still match the saved snapshot exactly; client-supplied display amounts never determine the charge.

The portal frontend needs to consume these additive fields before its receipt layout changes. This backend contract does not silently alter legacy client displays or accepted financial snapshots.
