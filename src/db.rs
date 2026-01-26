use chrono::{NaiveDateTime, Local};
use diesel::MultiConnection;
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool, CustomizeConnection, Error};
use diesel_migrations::*;
use std::path::Path;

use crate::config::TranspoConfig;


#[derive(MultiConnection)]
pub enum DbConnection {
    #[cfg(feature = "mysql")]
    Mysql(MysqlConnection),
    #[cfg(feature = "postgres")]
    Pg(PgConnection),
    #[cfg(feature = "sqlite")]
    Sqlite(SqliteConnection)
}

// Helper macro for doing DB stuff from async code.
// Requires `blocking::unblock` to be available in the current scope.
// Example usage:
// pool_unblock!(db_pool, c, Upload::set_is_completed(id, true, &mut c))
macro_rules! pool_unblock {
    ($pool:expr, $conn:ident, $($tail:tt)*) => {
        unblock(move || {
            let mut $conn = $pool.get().expect("Establishing database connection");
            pool_unblock!(_ $conn, $($tail)*)
        })
    };
    (_ $conn:ident, $($tail:tt)*) => {
        pool_unblock!($($tail)*)
    };
    ($expr:expr) => { $expr };
    ($block:block) => { $block };
    ($stmt:stmt) => { $stmt };
}
pub(crate) use pool_unblock;

#[derive(Debug)]
#[derive(Queryable)]
#[derive(Insertable)]
#[diesel(table_name = uploads)]
pub struct Upload {
    // unique identifier for this upload
    pub id: i64,
    pub password_hash: Option<Vec<u8>>,
    // number of remaining downloads (if download limit is enabled)
    pub remaining_downloads: Option<i32>,
    // deadline after which the upload expires
    pub expire_after: NaiveDateTime,
    // whether or not the upload has fully completed
    // used when reporting file size
    pub is_completed: bool
}

diesel::table! {
    uploads (id) {
        id -> BigInt,
        password_hash -> Nullable<Binary>,
        remaining_downloads -> Nullable<Integer>,
        expire_after -> Timestamp,
        is_completed -> Bool,
    }
}

impl Upload {
    // Insert into DB, return number of modified rows, or None if there
    // was a problem.
    pub fn insert(&self, c: &mut DbConnection) -> Option<usize> {
        let insert = diesel::insert_into(uploads::table)
            .values(self);
       
        insert.execute(c).ok()
    }

    // Return whether or not an Upload has expired, either based on time or
    // by depleting its maximum number of downloads
    pub fn is_expired(&self) -> bool {
        self.is_expired_time() || self.is_expired_downloads()
    }

    // Return whether or not the expiry date for an upload has been reached
    pub fn is_expired_time(&self) -> bool {
        let now = Local::now().naive_utc();
        now > self.expire_after
    }

    // Return whether or not the maximum downloads allowed on this upload have
    // have been expended
    pub fn is_expired_downloads(&self) -> bool {
        if let Some(remaining_downloads) = self.remaining_downloads {
            remaining_downloads <= 0
        } else {
            false
        }
    }

    // Return the Upload with the given ID
    pub fn select_with_id(id: i64, c: &mut DbConnection) -> Option<Self> {
        let select = uploads::table
            .filter(uploads::id.eq(id))
            .limit(1);

        select.load::<Upload>(c).ok()?.pop()
    }

    // Decrement the number of remaining downloads on the row with the given ID. Return
    // the number of modified rows.
    pub fn decrement_remaining_downloads(id: i64, c: &mut DbConnection) -> Option<usize> {
        let target = uploads::table
            .filter(uploads::id.eq(id)
                .and(uploads::remaining_downloads.is_not_null()));
        let update = diesel::update(target)
            .set(uploads::remaining_downloads.eq(uploads::remaining_downloads - 1));

        update.execute(c).ok()
    }

    pub fn set_is_completed(id: i64, is_completed: bool, c: &mut DbConnection) -> Option<usize> {
        let target = uploads::table
            .filter(uploads::id.eq(id));

        let update = diesel::update(target)
            .set(uploads::is_completed.eq(is_completed));

        update.execute(c).ok()
    }

    // Delete the row with the given ID
    pub fn delete_with_id(id: i64, c: &mut DbConnection) -> Option<usize> {
        let target = uploads::table
            .filter(uploads::id.eq(id));
        let delete = diesel::delete(target);

        delete.execute(c).ok()
    }

    // Return a list of IDs for expired (time-based) uploads
    pub fn select_expired(c: &mut DbConnection) -> Option<Vec<i64>> {
        let now = Local::now().naive_utc();
        let select = uploads::table
            .filter(uploads::expire_after.lt(now))
            .select(uploads::id);

        select.load::<i64>(c).ok()
    }

    pub fn select_all(c: &mut DbConnection) -> Option<Vec<i64>> {
        let select = uploads::table.select(uploads::id);

        select.load::<i64>(c).ok()
    }
}

pub fn run_migrations<P>(c: &mut DbConnection, path: P)
where P: AsRef<Path>
{
    let path = path.as_ref();
    let path = match c {
        #[cfg(feature = "mysql")]
        DbConnection::Mysql(_) => path.join("migrations"),

        #[cfg(feature = "sqlite")]
        DbConnection::Sqlite(_) => path.join("migrations"),

        #[cfg(feature = "postgres")]
        DbConnection::Pg(_) => path.join("pg_migrations")
    };

    let migrations = FileBasedMigrations::from_path(path)
        .expect("Opening DB migrations directory");

    let mut harness = HarnessWithOutput::write_to_stdout(c);
    harness.run_pending_migrations(migrations)
        .expect("Running database migrations");
}

#[derive(Debug)]
struct Customizer ();
impl CustomizeConnection<DbConnection, Error> for Customizer {
    fn on_acquire(&self, conn: &mut DbConnection) -> Result<(), Error> {
        #[allow(irrefutable_let_patterns)]
        #[cfg(feature = "sqlite")]
        if let DbConnection::Sqlite(conn) = conn {
            // Increase the busy timeout on sqlite connections to prevent getting
            // "database is locked" errors when there's many concurrent connections.
            let set = diesel::sql_query("PRAGMA busy_timeout = 15000;");
            set.execute(conn)?;
        }
        Ok(())
    }
}

pub type DbConnectionPool = Pool<ConnectionManager<DbConnection>>;
impl From<&TranspoConfig> for DbConnectionPool {
    fn from(config: &TranspoConfig) -> Self {
        let manager = ConnectionManager::<DbConnection>::new(&config.db_url);
        Pool::builder()
            .max_size(config.max_db_pool_size)
            .min_idle(config.min_db_pool_idle)
            .connection_customizer(Box::new(Customizer()))
            .build(manager)
            .expect("Creating database connection pool")
    }
}
