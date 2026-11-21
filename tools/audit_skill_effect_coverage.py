"""Report every accepted skill identity and consumer; unknown types fail the audit.
This is a coverage inventory, not a default-pass semantic test.
"""
from pathlib import Path
import argparse, hashlib, json

ROOT = Path(__file__).resolve().parents[1]
# Each entry is deliberate. Unresolved entries remain visible and counted.
CONTRACTS = {
 1: ("Damage", "implemented", "skills::handle_skill_cast; native damage20"),
 2: ("HealHP", "implemented", "maxHP permille; capped authoritative healing16"),
 4: ("Sleep", "implemented", "Freedom/Invulnerable block; damage wakes; client immobilizes"),
 5: ("Snare", "implemented", "native player absolute speed subtraction; OpenFusion NPC half speed"),
 8: ("Stun", "implemented", "Freedom/Invulnerable block; movement and attack gate"),
 10: ("Jump", "implemented", "Status.UpdateSkillBuff adds source value to base jump velocity"),
 11: ("Run", "implemented", "Status.UpdateSkillBuff adds source value; vehicle max before snare"),
 12: ("Stealth", "implemented", "condition; OpenFusion sight range /3; original VFX publication separate"),
 14: ("MiniMapEnemy", "implemented-continuation", "condition/icon plus ordinary server-interest enemy detector readout; original minimap styling not claimed"),
 15: ("MiniMapTreasure", "implemented-continuation", "condition/icon plus strict native shiny-interest treasure detector readout; original minimap styling not claimed"),
 16: ("Phoenix", "implemented", "authorized dead self revive; consumes active Nano only"),
 17: ("ProtectBattery", "implemented", "blocks accepted BatteryDrain skill"),
 18: ("ProtectInfection", "implemented", "infection blocked; Nano protection consumes3 stamina; shiny exception"),
 19: ("RewardBlob", "implemented", "local OpenFusion 20%/23%; combat/missions/racing, excludes eggs"),
 20: ("RewardCash", "implemented", "local OpenFusion 20%/23%; combat/missions/racing, excludes eggs"),
 21: ("BatteryDrain", "implemented", "weapon/Nano battery drain; ProtectBattery; exact40 ABI"),
 22: ("CorruptionAttack", "implemented", "corruption outcome handler; native20 damage or40 outcome"),
 23: ("InfectionDamage", "implemented", "poison condition; server periodic infection17 tick32"),
 25: ("Freedom", "implemented", "blocks crowd control; source-owned group passives"),
 26: ("PhoenixGroup", "implemented", "real group cast creates10s one-shot authorization; native12 result; regen4"),
 27: ("Recall", "implemented", "registered same-instance RXCom; verified server route integration"),
 28: ("RecallGroup", "implemented", "same-instance group routing; explicit registered Recall Point"),
 29: ("RetroRocketSelf", "external-dispatch", "weapon projectile path; no general Nano trailer"),
 30: ("BloodSucking", "implemented", "actual caster heal16 + target damage20; capped HP; native36 trailer"),
 31: ("BoundingBall", "implemented", "local OpenFusion2%maxHP per2s; native periodic damage20"),
 32: ("Invulnerable", "implemented", "damage and hostile debuffs blocked"),
 33: ("NanoStimPak", "implemented", "slot-specific21/22/23; native boosted source tier/cost"),
 34: ("ReturnHomeHeal", "implemented", "NPC retreat reset path and capped healing16"),
 35: ("BuffHeal", "unresolved", "flag24 + source duration/icon works; local OpenFusion has no heal proc; no proven A/B/C periodic formula"),
 36: ("ExtraBank", "unresolved", "flag25 mapping exists; item130 rejected before consumption until value500 capacity contract is proved; native distributor has no36 case"),
 40: ("Dash", "unresolved", "accepted IDs5/69/87 not loaded by Rust enum; absent local OpenFusion enum/native35-case distributor; movement contract not proved"),
}

def audit(xdt):
    skills = {}
    for row in xdt["m_pSkillTable"]["m_pSkillData"]:
        sid = row["m_iSkillNumber"]
        if sid <= 0: continue
        if sid in skills and skills[sid] != row: raise ValueError(f"Conflicting skill {sid}")
        skills[sid] = row
    unknown = {s["m_iSkillType"] for s in skills.values()} - CONTRACTS.keys()
    if unknown: raise ValueError(f"Unclassified accepted skill types: {sorted(unknown)}")
    uses = {sid: [] for sid in skills}
    def use(sid, kind, identity, field, runtime=None):
        if sid <= 0: return
        if sid not in skills: raise ValueError(f"Unknown skill {sid} referenced by {kind}{identity}/{field}")
        uses[sid].append({"kind":kind,"id":identity,"field":field, **({"runtime":runtime} if runtime else {})})
    for r in xdt['m_pNanoTable']['m_pNanoTuneData']: use(r['m_iSkillID'], 'nano-tune', r['m_iTuneNumber'], 'm_iSkillID')
    for r in xdt['m_pNpcTable']['m_pNpcData']:
        for field in ['m_iActiveSkill1','m_iActiveSkill2','m_iActiveSkill3','m_iPassiveBuff','m_iSupportSkill','m_iMegaType']:
            use(r[field], 'npc', r['m_iNpcNumber'], field)
    for r in xdt['m_pGeneralItemTable']['m_pItemData']:
        subtype=r['m_iItemType']; sid=r['m_iLinkSkill']
        runtime = 'declared-default-link/not-usable-subtype'
        if subtype==3: runtime='implemented-gumball'
        elif subtype==4: runtime='unresolved-capacity/rejected-unconsumed' if sid==189 else 'implemented-linked-consumable'
        elif subtype==10: runtime='unresolved-revive-token-use/rejected-unconsumed'
        elif subtype==11: runtime='unsupported-native0104-Nanocom-equipment/rejected-unconsumed'
        use(sid, 'general-item-subtype'+str(subtype), r['m_iItemNumber'], 'm_iLinkSkill', runtime)
    for r in xdt['m_pShinyTable']['m_pShinyData']: use(r['m_iSkillNumber'], 'shiny', r['m_iShinyID'], 'm_iSkillNumber')
    rows = []
    for sid, row in sorted(skills.items()):
        ty = row['m_iSkillType']; name, status, semantics = CONTRACTS[ty]
        rows.append({'id':sid,'type':ty,'name':name,'status':status,'semantics':semantics,'consumers':uses[sid],
          'passive':row['m_iBatteryDrainType']==2,'target_type':row['m_iTargetType'],
          'costs':row['m_iBatteryDrainUse'],'cooldown_tenths':row['m_iCoolTime'],'duration_tenths':row['m_iDurationTime']})
    return rows

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--output',type=Path,default=ROOT/'docs'/'skill-effect-coverage.json');args=parser.parse_args()
    source=ROOT/'tabledata'/'xdt.json';rows=audit(json.loads(source.read_text(encoding='utf-8')))
    report={'source':'tabledata/xdt.json','sha256':hashlib.sha256(source.read_bytes()).hexdigest(),
      'scope':'Static data/dispatch inventory; implemented is a contract claim, not runtime acceptance. Persistent original VFX not yet globally accepted.',
      'unique_skills':len(rows),'types':len({r['type'] for r in rows}),
      'unresolved_skills':[r['id'] for r in rows if r['status']=='unresolved'],
      'partial_skills':[r['id'] for r in rows if r['status']=='partial'],'unsupported_consumers':[c for r in rows for c in r['consumers'] if c.get('runtime','').startswith('unsupported')],'unresolved_consumers':[c for r in rows for c in r['consumers'] if c.get('runtime','').startswith('unresolved')],'skills':rows}
    args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(report,indent=2,ensure_ascii=False)+'\n',encoding='utf-8')
    print(json.dumps({k:v for k,v in report.items() if k!='skills'},ensure_ascii=False))
if __name__=='__main__': main()
