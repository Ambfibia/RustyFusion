//! Native ambient sets and timed mission prerequisites use accepted TableData indices.
use serde::{Deserialize,Serialize};
use crate::entity::{Player,Combatant};
#[derive(Debug)]
pub struct AmbientBarker {pub set:i32,pub lines:[String;4],pub conditions:BarkerConditions}
#[derive(Debug,Default,Deserialize)]
#[serde(default)]
pub struct BarkerConditions {
    #[serde(rename="_ffoneBarkerMinLevel")] min_level:i32,
    #[serde(rename="_ffoneBarkerMaxLevel")] max_level:i32,
    #[serde(rename="_ffoneBarkerCompletedMission")] completed:i32,
    #[serde(rename="_ffoneBarkerUncompletedMission")] uncompleted:i32,
    #[serde(rename="_ffoneBarkerEquippedItem")] equipped:i32,
    #[serde(rename="_ffoneBarkerUnequippedItem")] unequipped:i32,
    #[serde(rename="_ffoneBarkerItemType")] item_type:i32,
    #[serde(rename="_ffoneBarkerMinTaros")] min_taros:i32,
    #[serde(rename="_ffoneBarkerMaxTaros")] max_taros:i32,
    #[serde(rename="_ffoneBarkerMinFM")] min_fm:i32,
    #[serde(rename="_ffoneBarkerMaxFM")] max_fm:i32,
}
impl BarkerConditions {
    pub fn validate(&self)->Result<(),String>{
        if [self.min_level,self.max_level,self.completed,self.uncompleted,self.equipped,self.unequipped,self.item_type,self.min_taros,self.max_taros,self.min_fm,self.max_fm].iter().any(|n|*n<0)||self.completed>1024||self.uncompleted>1024||self.item_type>10||self.equipped>32767||self.unequipped>32767{return Err("Invalid ambient conditions".into());}
        for (min,max) in [(self.min_level,self.max_level),(self.min_taros,self.max_taros),(self.min_fm,self.max_fm)]{if max>0&&max<min{return Err("Invalid ambient condition range".into());}}
        if self.completed>0&&self.completed==self.uncompleted||self.equipped>0&&self.equipped==self.unequipped{return Err("Conflicting ambient conditions".into());}Ok(())
    }
    pub fn allows(&self,p:&Player)->bool {
        let range=|n:i64,min:i32,max:i32|n>=i64::from(min)&&(max==0||n<=i64::from(max));
        let equipped=|id|p.get_equipped().iter().flatten().any(|i|i32::from(i.id)==id&&i.ty as i32==self.item_type);
        range(i64::from(p.get_level()),self.min_level,self.max_level)&&range(i64::from(p.get_taros()),self.min_taros,self.max_taros)&&range(i64::from(p.get_fusion_matter()),self.min_fm,self.max_fm)
        &&(self.completed==0||p.mission_journal.is_mission_completed(self.completed).unwrap_or(false))&&(self.uncompleted==0||!p.mission_journal.is_mission_completed(self.uncompleted).unwrap_or(false))
        &&(self.equipped==0||equipped(self.equipped))&&(self.unequipped==0||!equipped(self.unequipped))
    }
}
#[derive(Debug,Default,Deserialize,Serialize)]
#[serde(default)]
pub struct RequiredBark {
    #[serde(rename="_ffoneRequiredBarker")] pub line:i32,
    #[serde(rename="_ffoneRequiredBarkerNpc")] pub npc:i32,
    #[serde(rename="_ffoneRequiredBarkerSeconds")] pub seconds:u32,
}
impl RequiredBark {
    pub fn validate(&self)->Result<(),String>{
        if !((self.line==0&&self.npc==0&&self.seconds==0)||(self.line>=4&&self.npc>0&&self.seconds>0&&self.seconds<=86400)){return Err("Incomplete ambient barker prerequisite".into());}Ok(())
    }
    pub fn allows(&self,player:&Player)->bool {self.line==0||player.recent_barks.get(&(self.npc,self.line)).is_some_and(|at|at.elapsed().as_secs_f64()<=f64::from(self.seconds))}
}
