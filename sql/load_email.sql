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
WHERE playerid = $1 AND msgindex = $2;
