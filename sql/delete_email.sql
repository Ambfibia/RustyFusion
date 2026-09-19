DELETE FROM emaildata
WHERE playerid = $1 AND msgindex = $2;
