SELECT epid, playerid, score, ringcount, "time", "timestamp"
FROM raceresults
WHERE epid = $1 AND playerid = $2
ORDER BY score DESC
LIMIT 1;
