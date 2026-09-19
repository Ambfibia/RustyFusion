UPDATE emaildata
SET
    readflag = $3,
    itemflag = $4,
    taros = $5
WHERE playerid = $1 AND msgindex = $2;
