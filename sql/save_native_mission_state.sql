INSERT INTO NativeMissionState (PlayerID, State) VALUES ($1, $2)
ON CONFLICT (PlayerID) DO UPDATE SET State = excluded.State;
