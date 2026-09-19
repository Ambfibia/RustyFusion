SELECT slot, id, "type", opt, timelimit
FROM emailitems
WHERE playerid = $1 AND msgindex = $2;
