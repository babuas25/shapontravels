# Passenger fare versus leg fare — 2026-09-10

## Result

The inspected Search/RePrice responses expose passenger-type fares for the whole offered journey, not separately priced passenger types for each leg. No per-leg monetary fare was found in their directions/segments. This is a statement about captured coverage, not every possible supplier response.

Offline inspection used all `*raw.json` files in private `.local/evidence/reprice-contract-20260910-5pax/`: 7,104 Search offer occurrences and 33 RePrice fare objects with populated pricing. Repeated Search inventory is counted repeatedly; failed RePrices without a fare object are excluded. No new network call was made.

- Every inspected fare has one `bookingComponents` entry, including return/multicity offers.
- Exact Decimal sums of `passengerFares[type].totalPrice × passengerCounts[type]` equal the offer's `totalPrice` for all 7,137 fare objects.
- Recursive inspection of directions/segments found only `fareBasisCode` and baggage `amount` among keys containing price/fare/tax/amount. Neither is a per-leg passenger monetary fare.
- The supplied API document describes passenger-type fares and explicitly warns that booking components are not necessarily one-to-one with directions. Its multicity mapping distinguishes requested routes from connecting segments.

## Concrete sanitized return example

`tests/fixtures/production/triplover-return.json`, first offer: SQ DAC→SIN and SIN→DAC, 2 adults + 1 child + 1 infant.

| Passenger type | Individual fare for the offered return | Count | Subtotal |
|---|---:|---:|---:|
| ADT | 54,121.33 | 2 | 108,242.66 |
| CHD | 42,692.06 | 1 | 42,692.06 |
| INF | 6,641.33 | 1 | 6,641.33 |
| Total | | 4 | 157,576.05 |

The sole booking component and top-level total both equal 157,576.05. There is no evidenced separate outbound ADT/CHD/INF amount and inbound ADT/CHD/INF amount. Dividing by two would invent prices.

With a hypothetical applicable fixed 500 rule under the existing whole-journey passenger calculation, four passengers add 2,000, making 159,576.05. This is a calculation illustration, not a newly configured rule or supplier selling quote.

## Latest user decisions

- Approved return scope: DAC→SIN→DAC uses an applicable DAC→SIN Admin markup rule as one journey for rule matching. A separate SIN→DAC one-way does not match that rule; it needs its own SIN→DAC rule for that specific-route treatment. Existing broader fallback rules are not cancelled by this instruction.
- Calculation is conditional: if genuine separately priced passenger-type fares exist for each leg, the user wants markup per leg. This supersedes the blanket prohibition on per-leg markup only when such coverage is verified. The inspected samples do not meet that condition; current whole-journey calculation remains applicable to them.
- Future per-leg support requires actual fare-to-leg/passenger mapping and reconciled totals. Multiple flight segments, fare-basis strings, baggage quantities, or multiple booking components alone do not prove this coverage. Do not split whole fares or count alternatives as additional purchased legs.
- The user has not approved the earlier mixed-airline plating-carrier or first-route multicity proposals. Those remain pending independently. Return scope approval does not select an airline rule for a mixed-airline fare.

No runtime implementation, database write, supplier mutation or release in this evidence review.
