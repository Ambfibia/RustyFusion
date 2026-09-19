SELECT COALESCE(MAX(msgindex), 0) + 1 AS nextindex
FROM emaildata
WHERE playerid = $1;
