DELETE FROM emailitems
WHERE playerid = $1 AND msgindex = $2 AND slot = $3;
