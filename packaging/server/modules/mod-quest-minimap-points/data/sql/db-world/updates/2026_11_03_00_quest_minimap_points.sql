-- mod-quest-minimap-points: fill quest_template's point (POIContinent, POIx, POIy) from the
-- quest's own quest_poi areas, for quests that have none.
--
-- A 3.3.5a client draws exactly one minimap marker for a quest, with an arrow at the minimap's
-- edge when it is off the map, and takes it only from this point (sent in the quest query
-- response). The shaded areas on the world map come from quest_poi and never reach the minimap.
-- Retail servers sent a point for most quests; AzerothCore's world database has one for about
-- a hundred, so the marker and its arrow are almost always missing.
--
-- The point is the centre of the quest's largest objective area (most polygon vertices, then the
-- lowest area id), or of its hand-in area when it has no objective area. Against the quests that
-- still carry Blizzard's point this lands within 50 yards of it for 81 of 104, within 150 for 92.
-- Only quests whose point is (0, 0) are changed, so curated points and edits are kept.
UPDATE `quest_template` AS t
JOIN (
    SELECT `q`, `m`, `x`, `y`
    FROM (
        SELECT a.*, ROW_NUMBER() OVER (PARTITION BY a.`q` ORDER BY a.`oi` < 0, a.`n` DESC, a.`id`) AS `r`
        FROM (
            SELECT p.`QuestID` AS `q`, p.`id`, p.`ObjectiveIndex` AS `oi`, p.`MapID` AS `m`,
                   COUNT(*) AS `n`, AVG(pp.`X`) AS `x`, AVG(pp.`Y`) AS `y`
            FROM `quest_poi` AS p
            JOIN `quest_poi_points` AS pp ON pp.`QuestID` = p.`QuestID` AND pp.`Idx1` = p.`id`
            GROUP BY p.`QuestID`, p.`id`, p.`ObjectiveIndex`, p.`MapID`
        ) AS a
    ) AS ranked
    WHERE `r` = 1
) AS best ON best.`q` = t.`ID`
SET t.`POIContinent` = best.`m`, t.`POIx` = best.`x`, t.`POIy` = best.`y`
WHERE t.`POIx` = 0 AND t.`POIy` = 0;
