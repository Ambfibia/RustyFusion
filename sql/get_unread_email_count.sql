SELECT COUNT(*) AS unreadcount
FROM emaildata
WHERE playerid = $1 AND readflag = 0;
