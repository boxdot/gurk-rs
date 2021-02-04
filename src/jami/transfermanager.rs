use app_dirs::{get_app_dir, AppDataType, AppInfo};
use serde::{Deserialize, Serialize};

/**
 * A class used to store transfers per account per conversation
 **/
#[derive(Serialize, Deserialize)]
pub struct TransferManager {
    path: String,
}

impl TransferManager {
    /**
     * Generate a new TransferManager
     * @return the new manager
     */
    pub fn new() -> Self {
        let db_path = get_app_dir(
            AppDataType::UserData,
            &AppInfo {
                name: "jami",
                author: "SFL",
            },
            "jami-cli.db",
        );

        let path = db_path.unwrap().into_os_string().into_string().unwrap();
        let conn = rusqlite::Connection::open(&*path).unwrap();
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap_or(0);
        let do_migration = version != 1;
        if do_migration {
            conn.execute("CREATE TABLE IF NOT EXISTS transfers (
                id               INTEGER PRIMARY KEY,
                account_id       TEXT,
                conversation_id  TEXT,
                tid              TEXT,
                path             TEXT
                )", rusqlite::NO_PARAMS).unwrap();
            conn.pragma_update(None, "user_version", &1).unwrap();
        }

        Self {
            path
        }
    }

    pub fn path(&mut self, account_id: String, conv_id: String, tid: String) -> Option<String> {
        let conn = rusqlite::Connection::open(&*self.path).unwrap();
        let mut stmt = conn.prepare("SELECT path FROM transfers WHERE account_id=:account_id AND conversation_id=:conversation_id AND tid=:tid").unwrap();
        let mut rows = stmt.query_named(&[(":account_id", &account_id), (":conversation_id", &conv_id), (":tid", &tid)]).unwrap();
        if let Ok(Some(row)) = rows.next() {
            return match row.get(0) {
                Ok(r) => Some(r),
                _ => None,
            };
        }
        None
    }

    pub fn set_file_path(&mut self, account_id: String, conv_id: String, tid: String, path: String) -> Option<i32> {
        let conn = rusqlite::Connection::open(&*self.path).unwrap();
        // Else insert!
        let mut conn = conn.prepare("INSERT INTO transfers (account_id, conversation_id, tid, path)
                                     VALUES (:account_id, :conversation_id, :tid, :path)").unwrap();
        match conn.execute_named(&[(":account_id", &account_id),
                                   (":conversation_id", &conv_id),
                                   (":tid", &tid),
                                   (":path", &path)]) {
            Ok(id) => {
                return Some(id as i32);
            }
            Err(_e) => {
                return None;
            }
        }
    }

}
