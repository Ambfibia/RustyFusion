--[[
    Lord Fuse.

    Fuse fights in three bodies. Each of the first two, on death, summons the
    next body in place plus one of his arms off to the side. The final body is
    the end of the fight, so it just dies like any other mob.

    Stage bodies:  2466 (Adaptium) -> 2467 (Blastons) -> 2468 (Cosmix)
    Arms:          2469 (right, with stage 2), 2470 (left, with stage 3)

    Fuse himself never moves, so this script skips the roaming behavior that
    ordinary mobs use.
]]

local b = require("@lib/behaviors")

local STAGE_ONE = 2466
local STAGE_TWO = 2467

-- Which body and arm each stage summons when it falls, and where the arm goes
-- relative to Fuse.
local NEXT_STAGE = {
    [STAGE_ONE] = { body = 2467, arm = 2469, arm_offset_x = -600 },
    [STAGE_TWO] = { body = 2468, arm = 2470, arm_offset_x = 600 },
}

local function advance_stage(npc: Npc)
    local stage = NEXT_STAGE[npc:ty()]
    if not stage then
        return
    end

    log(`Lord Fuse advancing to body {stage.body}`)
    npc:summon_npc(stage.body, 0, 0, 0)
    npc:summon_npc(stage.arm, stage.arm_offset_x, 0, 0)
end

return function(npc: Npc)
    local advanced = false

    while true do
        if npc:is_dead() then
            if not advanced then
                -- summon the next stage before the death animation starts, so
                -- the fight never has a gap with nothing to shoot at
                advance_stage(npc)
                advanced = true
            end

            npc:begin_death()
            wait(2.0)
            npc:despawn()

            -- A stage body is done once it's defeated; the next stage has
            -- already taken over. Ending the script leaves the corpse
            -- despawned rather than respawning it on a timer.
            return
        end

        if not b.handle_sleep(npc) then
            b.scan_for_target_aggro(npc)

            local target = npc:target()
            if target then
                -- Fuse is rooted in place, so he only ever attacks what walks
                -- into his reach
                b.try_attack(npc)
            end
        end

        yield()
    end
end
