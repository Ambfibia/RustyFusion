INSERT INTO raceresults (
    epid,
    playerid,
    score,
    ringcount,
    "time",
    "timestamp"
)
VALUES (
    $1,
    $2,
    $3,
    $4,
    $5,
    $6
);
