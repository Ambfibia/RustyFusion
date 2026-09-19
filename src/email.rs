use std::time::SystemTime;

use crate::{
    defines::*,
    entity::Player,
    error::{FFError, FFResult, Severity},
    item::Item,
    net::packet::*,
    util,
};

/// Number of emails the client shows on one page of the inbox.
pub const EMAIL_PAGE_SIZE: usize = SIZEOF_EMAIL_PAGE_SIZE as usize;
/// Number of item attachment slots on a single email.
pub const EMAIL_ITEM_SLOTS: usize = SIZEOF_EMAIL_ITEM_CNT as usize;

/// A single email, as stored in the database.
///
/// `msg_index` is per-recipient and 1-based; it's what the client uses to
/// address an email in every follow-up request.
#[derive(Debug, Clone)]
pub struct Email {
    pub pc_uid: i64,
    pub msg_index: i32,
    pub read: bool,
    pub sender_uid: i64,
    pub sender_first_name: String,
    pub sender_last_name: String,
    pub subject: String,
    pub body: String,
    pub taros: u32,
    pub send_time: SystemTime,
    pub delete_time: Option<SystemTime>,
    pub attachments: [Option<Item>; EMAIL_ITEM_SLOTS],
}
impl Email {
    pub fn new(sender: &Player, recipient_uid: i64) -> Self {
        Self {
            pc_uid: recipient_uid,
            msg_index: 0, // assigned by the DB on insert
            read: false,
            sender_uid: sender.get_uid(),
            sender_first_name: sender.first_name.clone(),
            sender_last_name: sender.last_name.clone(),
            subject: String::new(),
            body: String::new(),
            taros: 0,
            send_time: SystemTime::now(),
            delete_time: None,
            attachments: [None; EMAIL_ITEM_SLOTS],
        }
    }

    /// The client's "has goodies" flag. It's derived, never stored authoritatively,
    /// so that claiming the last attachment clears the paperclip icon.
    pub fn has_attachments(&self) -> bool {
        self.taros > 0 || self.attachments.iter().any(|item| item.is_some())
    }

    pub fn get_sender_name(&self) -> String {
        format!("{} {}", self.sender_first_name, self.sender_last_name)
    }

    pub fn to_email_info(&self) -> FFResult<sEmailInfo> {
        Ok(sEmailInfo {
            iEmailIndex: self.msg_index as i64,
            iFromPCUID: self.sender_uid,
            szFirstName: util::encode_utf16(&self.sender_first_name)?,
            szLastName: util::encode_utf16(&self.sender_last_name)?,
            szSubject: util::encode_utf16(&self.subject)?,
            iReadFlag: self.read as i32,
            SendTime: timestamp_to_struct(self.send_time),
            DeleteTime: match self.delete_time {
                Some(time) => timestamp_to_struct(time),
                None => sSYSTEMTIME::default(),
            },
            iItemCandyFlag: self.has_attachments() as i32,
        })
    }

    pub fn get_attachment_slot(&self, slot_num: usize) -> FFResult<&Option<Item>> {
        // email item slots are 1-indexed on the wire
        if slot_num == 0 || slot_num > EMAIL_ITEM_SLOTS {
            return Err(FFError::build(
                Severity::Warning,
                format!("Bad email attachment slot {}", slot_num),
            ));
        }
        Ok(&self.attachments[slot_num - 1])
    }

    pub fn take_attachment(&mut self, slot_num: usize) -> FFResult<Option<Item>> {
        if slot_num == 0 || slot_num > EMAIL_ITEM_SLOTS {
            return Err(FFError::build(
                Severity::Warning,
                format!("Bad email attachment slot {}", slot_num),
            ));
        }
        Ok(self.attachments[slot_num - 1].take())
    }
}

/// Total taro cost of sending an email with `num_attachments` items and
/// `taros_attached` taros riding along.
pub fn get_email_cost(taros_attached: u32, num_attachments: usize) -> u32 {
    taros_attached + EMAIL_AND_MONEY_COST + EMAIL_ITEM_COST * num_attachments as u32
}

pub fn timestamp_to_struct(time: SystemTime) -> sSYSTEMTIME {
    use chrono::{DateTime, Datelike as _, Local, Timelike as _};
    let time: DateTime<Local> = time.into();
    sSYSTEMTIME {
        wYear: time.year(),
        wMonth: time.month() as i32,
        wDayOfWeek: time.weekday().num_days_from_sunday() as i32,
        wDay: time.day() as i32,
        wHour: time.hour() as i32,
        wMinute: time.minute() as i32,
        wSecond: time.second() as i32,
        wMilliseconds: time.timestamp_subsec_millis() as i32,
    }
}
