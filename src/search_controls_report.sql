WITH events AS MATERIALIZED (
 SELECT e.*, (SELECT count(*) FROM search_supplier_usage s WHERE s.search_id=e.id AND s.dispatched) hits
 FROM search_usage e
 WHERE started_at >= ($1::date::timestamp AT TIME ZONE 'Asia/Dhaka')
 AND started_at < (($2::date+1)::timestamp AT TIME ZONE 'Asia/Dhaka')
), totals AS (
 SELECT jsonb_build_object(
 'requestCount',count(*),'supplierApiHitCount',coalesce(sum(hits),0),
 'successCount',count(*) FILTER(WHERE outcome='success'),'failedCount',count(*) FILTER(WHERE outcome='failed'),
 'blockedCount',count(*) FILTER(WHERE outcome='blocked'),'anonymousCount',count(*) FILTER(WHERE subject IS NULL),
 'uniqueUserCount',count(DISTINCT subject),'averageTotalMs',round(avg(total_ms))) value FROM events
), people AS (
 SELECT u.clerk_user_id subject, nullif(trim(concat_ws(' ',u.first_name,u.last_name)),'') name,u.email,u.role,
 a.agency_code, p.fields->>'agencyName' agency_name
 FROM portal_users u LEFT JOIN portal_agency_memberships m ON m.user_id=u.id
 LEFT JOIN portal_agencies a ON a.id=m.agency_id
 LEFT JOIN portal_identity_profiles p ON p.user_id=a.owner_user_id AND p.kind='profile'
 UNION ALL SELECT DISTINCT e.subject,NULL,NULL,NULL,NULL,NULL FROM events e
 WHERE e.subject IS NULL OR NOT EXISTS(SELECT 1 FROM portal_users u WHERE u.clerk_user_id=e.subject)
), users AS (
 SELECT jsonb_build_object(
 'userId',p.subject,'displayName',p.name,'email',p.email,'role',p.role,'agencyCode',p.agency_code,'agencyName',p.agency_name,
 'searchEnabled',coalesce(c.search_enabled,true),'dailyLimit',c.daily_limit,'controlVersion',coalesce(c.version,0),
 'todayHitCount',coalesce((SELECT hits FROM search_daily_hits WHERE day=(clock_timestamp() AT TIME ZONE 'Asia/Dhaka')::date AND kind='actor' AND key=p.subject),0),
 'requestCount',stats.requests,'supplierApiHitCount',stats.hits,'successCount',stats.successes,'failedCount',stats.failures,'blockedCount',stats.blocked,
 'lastSearchAt',stats.latest,'averageTotalMs',stats.average,
 'searchedRoutes',coalesce(routes.items,'[]'::jsonb),'distinctRouteCount',routes.n
 ) value, stats.latest
 FROM people p LEFT JOIN search_user_controls c ON c.subject=p.subject
 CROSS JOIN LATERAL (
 SELECT count(*) requests,coalesce(sum(e.hits),0) hits,count(*) FILTER(WHERE outcome='success') successes,
 count(*) FILTER(WHERE outcome='failed') failures,count(*) FILTER(WHERE outcome='blocked') blocked,max(started_at) latest,round(avg(total_ms)) average
 FROM events e WHERE e.subject IS NOT DISTINCT FROM p.subject
 ) stats
 CROSS JOIN LATERAL (
 SELECT jsonb_agg(jsonb_build_object('route',r.route,'departureDates',r.dates,'requestCount',r.n,'lastSearchedAt',r.latest) ORDER BY r.latest DESC) items,count(*) n
 FROM (
 SELECT string_agg_route.route,string_agg_route.dates,count(*) n,max(e.started_at) latest FROM events e
 CROSS JOIN LATERAL (
 SELECT string_agg(leg->>'origin'||' → '||(leg->>'destination'),' / ' ORDER BY ord) route,
 jsonb_agg(leg->>'departureDate' ORDER BY ord) dates FROM jsonb_array_elements(e.routes) WITH ORDINALITY l(leg,ord)
 ) string_agg_route WHERE e.subject IS NOT DISTINCT FROM p.subject
 GROUP BY string_agg_route.route,string_agg_route.dates
 ) r
 ) routes
), suppliers AS (
 SELECT jsonb_build_object('supplier',l.supplier,'dailyLimit',l.daily_limit,'limitVersion',l.version,
 'todayHitCount',coalesce((SELECT hits FROM search_daily_hits WHERE day=(clock_timestamp() AT TIME ZONE 'Asia/Dhaka')::date AND kind='supplier' AND key=l.supplier),0),
 'requestCount',count(s.search_id),'supplierApiHitCount',count(*) FILTER(WHERE s.dispatched),
 'successCount',count(*) FILTER(WHERE s.outcome='success'),'failedCount',count(*) FILTER(WHERE s.outcome='failed'),
 'blockedCount',count(*) FILTER(WHERE s.outcome='blocked'),'lastSearchAt',max(e.started_at)) value
 FROM search_supplier_limits l LEFT JOIN (search_supplier_usage s JOIN events e ON e.id=s.search_id) ON s.supplier=l.supplier
 GROUP BY l.supplier
)
SELECT jsonb_build_object('available',true,'totals',(SELECT value FROM totals),
 'users',coalesce((SELECT jsonb_agg(value ORDER BY latest DESC NULLS LAST,value->>'userId') FROM users),'[]'::jsonb),
 'suppliers',coalesce((SELECT jsonb_agg(value ORDER BY value->>'supplier') FROM suppliers),'[]'::jsonb))
