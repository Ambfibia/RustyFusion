SELECT
    msgindex,
    readflag,
    senderid,
    senderfirstname,
    senderlastname,
    subjectline,
    msgbody,
    taros,
    sendtime,
    deletetime
FROM emaildata
WHERE playerid = $1
ORDER BY msgindex DESC
LIMIT 5
OFFSET $2;
