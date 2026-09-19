INSERT INTO emaildata (
    playerid,
    msgindex,
    readflag,
    itemflag,
    senderid,
    senderfirstname,
    senderlastname,
    subjectline,
    msgbody,
    taros,
    sendtime,
    deletetime
)
VALUES (
    $1,
    $2,
    $3,
    $4,
    $5,
    $6,
    $7,
    $8,
    $9,
    $10,
    $11,
    $12
);
