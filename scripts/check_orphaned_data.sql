-- =============================================
-- TrendCart: Check for Orphaned Data
-- รันบน production DB ก่อนและหลัง deploy เพื่อหา data integrity issues
-- =============================================

-- 1. Products ที่ creator_id ไม่มีอยู่
SELECT 'orphaned_products' AS issue, COUNT(*) AS count
FROM products p
WHERE NOT EXISTS (SELECT 1 FROM creators c WHERE c.id = p.creator_id);

-- 2. Merchant links ที่ product_id ไม่มีอยู่
SELECT 'orphaned_merchant_links' AS issue, COUNT(*) AS count
FROM merchant_links ml
WHERE NOT EXISTS (SELECT 1 FROM products p WHERE p.id = ml.product_id);

-- 3. Product variants ที่ merchant_link_id ไม่มีอยู่
SELECT 'orphaned_product_variants' AS issue, COUNT(*) AS count
FROM product_variants pv
WHERE NOT EXISTS (SELECT 1 FROM merchant_links ml WHERE ml.id = pv.merchant_link_id);

-- 4. Redirect events ที่ merchant_link_id ไม่มีอยู่
SELECT 'orphaned_redirect_events' AS issue, COUNT(*) AS count
FROM redirect_events re
WHERE NOT EXISTS (SELECT 1 FROM merchant_links ml WHERE ml.id = re.merchant_link_id);

-- 5. Conversions ที่ click_id ไม่มีอยู่ใน redirect_events
SELECT 'orphaned_conversions_click' AS issue, COUNT(*) AS count
FROM conversions c
WHERE NOT EXISTS (SELECT 1 FROM redirect_events re WHERE re.click_id = c.click_id);

-- 6. Conversions ที่ merchant_link_id ไม่มีอยู่
SELECT 'orphaned_conversions_merchant' AS issue, COUNT(*) AS count
FROM conversions c
WHERE NOT EXISTS (SELECT 1 FROM merchant_links ml WHERE ml.id = c.merchant_link_id);

-- 7. Conversions ที่ creator_id ไม่มีอยู่
SELECT 'orphaned_conversions_creator' AS issue, COUNT(*) AS count
FROM conversions c
WHERE NOT EXISTS (SELECT 1 FROM creators cr WHERE cr.id = c.creator_id);

-- 8. Scrape sources ที่ creator_id ไม่มีอยู่
SELECT 'orphaned_scrape_sources' AS issue, COUNT(*) AS count
FROM scrape_sources ss
WHERE NOT EXISTS (SELECT 1 FROM creators c WHERE c.id = ss.creator_id);

-- 9. Scrape jobs ที่ source_id ไม่มีอยู่
SELECT 'orphaned_scrape_jobs' AS issue, COUNT(*) AS count
FROM scrape_jobs sj
WHERE NOT EXISTS (SELECT 1 FROM scrape_sources ss WHERE ss.id = sj.source_id);

-- =============================================
-- ถ้าเจอ count > 0 ให้ดูรายละเอียด:
-- SELECT * FROM products WHERE NOT EXISTS (SELECT 1 FROM creators WHERE id = products.creator_id);
-- =============================================
