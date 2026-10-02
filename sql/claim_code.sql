INSERT INTO RedeemedCodes (PlayerID, Code) VALUES ($1, $2)
ON CONFLICT (PlayerID, Code) DO NOTHING;
