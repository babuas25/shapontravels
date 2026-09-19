WITH records AS (
 SELECT b.id,b.public_ref reference,coalesce(nullif(b.pnr,''),'—') pnr,
 coalesce(r.selling#>>'{item1,platingCarrier}','—') airline,
 coalesce((SELECT sum(jsonb_array_length(x#>'{0,segments}')) FROM jsonb_array_elements(r.selling#>'{item1,directions}') x),0) segments,
 b.created_at,coalesce(v.created_at,t.updated_at) ticketed_at,
 jsonb_array_length(b.request->'passengerInfoes') pax,
 upper(concat_ws(' ', b.request#>>'{passengerInfoes,0,nameElement,title}', b.request#>>'{passengerInfoes,0,nameElement,firstName}',b.request#>>'{passengerInfoes,0,nameElement,lastName}')) name,
 r.currency, (r.tier_pricing->>'gross')::numeric gross,(r.tier_pricing->>'payable')::numeric payable,
 b.created_by_external_user_id creator,
 coalesce(nullif(trim(concat_ws(' ',u.first_name,u.last_name)),''),u.email,'Unknown user') creator_name
 FROM flight_bookings b JOIN flight_reprices r ON r.id=b.price_id
 JOIN wallet_client_links cl ON cl.client_id=b.client_id JOIN wallet_owners o ON o.id=cl.owner_id AND o.owner_type='agency' AND o.owner_key=$1
 JOIN flight_ticket_issues t ON t.booking_id=b.id LEFT JOIN flight_ticket_verifications v ON v.issue_id=t.id
 LEFT JOIN portal_users u ON u.clerk_user_id=b.created_by_external_user_id
 WHERE (t.state='issued' OR v.issue_id IS NOT NULL) AND coalesce(v.created_at,t.updated_at)<=$9
 UNION ALL
 SELECT b.id,coalesce(b.booking_reference,b.public_ref),coalesce(b.data->>'pnr','—'),coalesce(b.data#>>'{itinerary,carrierCode}','—'),
 coalesce((SELECT sum(jsonb_array_length(l->'segments')) FROM jsonb_array_elements(b.data#>'{itinerary,legs}') l),0),
 b.created_at,b.issued_at,jsonb_array_length(b.data#>'{passengers,travellers}'),
 upper(concat_ws(' ',b.data#>>'{passengers,travellers,0,title}',b.data#>>'{passengers,travellers,0,firstName}',b.data#>>'{passengers,travellers,0,lastName}')),
 b.currency,b.gross_minor::numeric/100,b.payable_minor::numeric/100,u.clerk_user_id,coalesce(nullif(trim(concat_ws(' ',u.first_name,u.last_name)),''),u.email,'Unknown user')
 FROM portal_import_bookings b JOIN portal_users u ON u.id=b.creator_id WHERE b.agency_code=$1 AND b.status='confirmed' AND b.issued_at<=$9 AND b.updated_at<=$9
), filtered AS (
 SELECT * FROM records WHERE ($2='' OR ticketed_at >= nullif($2,'')::date AT TIME ZONE 'UTC')
 AND ($3='' OR ticketed_at < (nullif($3,'')::date+1) AT TIME ZONE 'UTC')
 AND ($4='' OR creator=$4) AND ($5='' OR airline=$5) AND ($6='' OR position(lower($6) IN lower(concat_ws(' ',reference,pnr,name)))>0)
), page AS (SELECT * FROM filtered ORDER BY ticketed_at DESC,id DESC LIMIT $7 OFFSET $8), currencies AS (
 SELECT currency,coalesce(sum(gross),0) gross,coalesce(sum(payable),0) payable,coalesce(sum(gross-payable),0) profit FROM filtered GROUP BY currency
)
SELECT jsonb_build_object('asOf',$9,'total',(SELECT count(*) FROM filtered),'rows',coalesce((SELECT jsonb_agg(jsonb_build_object(
 'id',id,'orderReference',reference,'pnr',pnr,'airlineCode',airline,'airlineName',airline,'totalSegments',segments,'createdAt',created_at,'ticketedAt',ticketed_at,'totalPassengers',pax,'mainTravellerName',name,'currency',currency,'grossFare',gross,'payableAmount',payable,'profit',gross-payable,'bookedByUserId',creator,'bookedByName',creator_name) ORDER BY ticketed_at DESC,id DESC) FROM page),'[]'::jsonb),
 'users',coalesce((SELECT jsonb_agg(jsonb_build_object('id',id,'label',label) ORDER BY label) FROM (SELECT DISTINCT creator id,creator_name label FROM records WHERE creator IS NOT NULL UNION SELECT u.clerk_user_id,coalesce(nullif(trim(concat_ws(' ',u.first_name,u.last_name)),''),u.email,u.clerk_user_id) FROM portal_users u JOIN portal_agency_memberships m ON m.user_id=u.id JOIN portal_agencies a ON a.id=m.agency_id WHERE a.agency_code=$1 AND u.status='active') u),'[]'::jsonb),
 'airlines',coalesce((SELECT jsonb_agg(jsonb_build_object('code',airline,'label',airline) ORDER BY airline) FROM (SELECT DISTINCT airline FROM records) a),'[]'::jsonb),
 'summary',jsonb_build_object('totalItems',(SELECT count(*) FROM filtered),'totalPassengers',(SELECT coalesce(sum(pax),0) FROM filtered),'missingGrossCount',(SELECT count(*) FROM filtered WHERE gross IS NULL),'totalsByCurrency',coalesce((SELECT jsonb_agg(jsonb_build_object('currency',currency,'totalGross',gross,'totalPayable',payable,'totalProfit',profit) ORDER BY currency) FROM currencies),'[]'::jsonb)))
