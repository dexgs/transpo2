use diesel::prelude::*;
use diesel_migrations::*;
use chrono::{NaiveDateTime, Local};

embed_migrations!();

pub enum DbConnection {
    #[cfg(feature = "mysql")]
    Mysql(MysqlConnection),
    #[cfg(feature = "postgres")]
    Pg(PgConnection),
    #[cfg(feature = "sqlite")]
    Sqlite(SqliteConnection)
}

#[derive(Clone, Copy)]
pub enum DbBackend {
    #[cfg(feature = "mysql")]
    Mysql,
    #[cfg(feature = "postgres")]
    Pg,
    #[cfg(feature = "sqlite")]
    Sqlite
}

#[derive(Debug)]
#[derive(Queryable)]
#[derive(Insertable)]
#[table_name="uploads"]
pub struct Upload {
    pub id: i64,
    pub file_name: String,
    pub mime_type: String,
    pub password_hash: Option<Vec<u8>>,
    pub remaining_downloads: Option<i32>,
    pub num_accessors: i32,
    pub expire_after: NaiveDateTime
}

table! {
    uploads (id) {
        id -> BigInt,
        file_name -> Text,
        mime_type -> Text,
        password_hash -> Nullable<Binary>,
        remaining_downloads -> Nullable<Integer>,
        num_accessors -> Integer,
        expire_after -> Timestamp,
    }
}

impl Upload {
    // Insert into DB, return number of modified rows, or None if there
    // was a problem.
    pub fn insert(&self, db_connection: &DbConnection) -> Option<usize> {
        let insert = diesel::insert_into(uploads::table)
            .values(self);
        
        match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::Mysql(c) => insert.execute(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => insert.execute(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => insert.execute(c),
        }.ok()
    }

    // Return whether or not an Upload has expired, either based on time or
    // by depleting its maximum number of downloads
    pub fn is_expired(&self) -> bool {
        self.is_expired_time() || self.is_expired_downloads()
    }

    // Return whether or not the expiry date for an upload has been reached
    pub fn is_expired_time(&self) -> bool {
        let now = Local::now().naive_local();
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
    pub fn select_with_id(id: i64, db_connection: &DbConnection) -> Option<Self> {
        let select = uploads::table
            .filter(uploads::id.eq(id))
            .limit(1);

        match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::MySql(c) => select.load::<Upload>(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => select.load::<Upload>(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => select.load::<Upload>(c)
        }.ok()?.pop()
    }

    // Decrement the number of remaining downloads on the row with the given ID. Return
    // the number of modified rows.
    pub fn decrement_remaining_downloads(id: i64, db_connection: &DbConnection) -> Option<usize> {
        let target = uploads::table
            .filter(uploads::id.eq(id)
                .and(uploads::remaining_downloads.is_not_null()));
        let update = diesel::update(target)
            .set(uploads::remaining_downloads.eq(uploads::remaining_downloads - 1));

        match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::MySql(c) => update.execute(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => update.execute(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => update.execute(c)
        }.ok()
    }

    // Delete the row with the given ID
    pub fn delete_with_id(id: i64, db_connection: &DbConnection) -> Option<usize> {
        let target = uploads::table
            .filter(uploads::id.eq(id));
        let delete = diesel::delete(target);

        match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::MySql(c) => delete.execute(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => delete.execute(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => delete.execute(c)
        }.ok()
    }

    // Return a list of IDs for expired (time-based) uploads
    pub fn select_expired(db_connection: &DbConnection) -> Option<Vec<i64>> {
        let now = Local::now().naive_local();
        let select = uploads::table
            .filter(uploads::expire_after.lt(now))
            .select(uploads::id);

        match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::MySql(c) => select.load::<i64>(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => select.load::<i64>(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => select.load::<i64>(c)
        }.ok()
    }

    // Increment the accessor count
    pub fn access(db_connection: &DbConnection, id: i64) -> Option<usize> {
        let target = uploads::table
            .filter(uploads::id.eq(id));
        let update = diesel::update(target)
            .set(uploads::num_accessors.eq(uploads::num_accessors + 1));

        match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::MySql(c) => update.execute(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => update.execute(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => update.execute(c)
        }.ok()
    }

    // Decrement the accessor count
    pub fn revoke(db_connection: &DbConnection, id: i64) -> Option<usize> {
        let target = uploads::table
            .filter(uploads::dsl::id.eq(id));
        let update = diesel::update(target)
            .set(uploads::dsl::num_accessors.eq(uploads::dsl::num_accessors - 1));

        match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::MySql(c) => update.execute(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => update.execute(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => update.execute(c)
        }.ok()
    }

    pub fn num_accessors(db_connection: &DbConnection, id: i64) -> Option<i32> {
        let select = uploads::table
            .filter(uploads::dsl::id.eq(id))
            .select(uploads::dsl::num_accessors);

        match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::MySql(c) => select.load::<i32>(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => select.load::<i32>(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => select.load::<i32>(c)
        }.ok()?.pop()
    }
}


#[derive(Queryable)]
#[derive(Insertable)]
#[table_name="storage_size"]
pub struct StorageSize {
    pub id: i32,
    pub num_bytes: i64
}

table! {
    storage_size (id) {
        id -> Integer,
        num_bytes -> BigInt,
    }
}

impl StorageSize {
    fn insert(&self, db_connection: &DbConnection) -> Option<usize> {
        let insert = diesel::insert_into(storage_size::table)
            .values(self);

        match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::Mysql(c) => insert.execute(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => insert.execute(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => insert.execute(c),
        }.ok()
    }

    pub fn set(db_connection: &DbConnection, size: i64) -> Option<usize> {
        let target = storage_size::table
            .filter(storage_size::id.eq(0));
        let update = diesel::update(target)
            .set(storage_size::num_bytes.eq(size));

        let update_result = match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::MySql(c) => update.execute(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => update.execute(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => update.execute(c)
        }.ok();

        if let Some(1) = update_result {
            update_result
        } else {
            let storage_size = Self {
                id: 0,
                num_bytes: size
            };

            storage_size.insert(db_connection)
        }
    }

    pub fn increment(db_connection: &DbConnection, size: i64) -> Option<usize> {
        let target = storage_size::table
            .filter(storage_size::id.eq(0));
        let update = diesel::update(target)
            .set(storage_size::num_bytes.eq(storage_size::num_bytes + size));

        match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::MySql(c) => update.execute(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => update.execute(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => update.execute(c)
        }.ok()
    }

    pub fn get(db_connection: &DbConnection) -> Option<i64> {
        let select = storage_size::table
            .filter(storage_size::id.eq(0))
            .select(storage_size::num_bytes)
            .limit(1);

        match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::MySql(c) => select.load::<i64>(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => select.load::<i64>(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => select.load::<i64>(c)
        }.ok()?.pop()
    }
}


pub fn run_migrations(db_connection: &DbConnection) {
    let stdout = &mut std::io::stdout();
    match db_connection {
        #[cfg(feature = "mysql")]
        DbConnection::Mysql(c) =>
            embedded_migrations::run_with_output(c, stdout),

            #[cfg(feature = "postgres")]
        DbConnection::Pg(c) =>
            embedded_migrations::run_with_output(c, stdout),

            #[cfg(feature = "sqlite")]
        DbConnection::Sqlite(c) =>
            embedded_migrations::run_with_output(c, stdout)
    }.expect("Running database migrations");
}

pub fn parse_db_backend(db_url: &str) -> Option<DbBackend> {
    if db_url.starts_with("mysql://") {
        #[cfg(feature = "mysql")]
        return Some(DbBackend::Mysql);
    } else if db_url.starts_with("postgresql://") {
        #[cfg(feature = "postgres")]
        return Some(DbBackend::Pg);
    } else {
        #[cfg(feature = "sqlite")]
        return Some(DbBackend::Sqlite);
    }

    None
}

pub fn establish_connection(db_backend: DbBackend, db_url: &str) -> DbConnection {
    match db_backend {
        #[cfg(feature = "mysql")]
        DbBackend::Mysql => DbConnection::Mysql(
            MysqlConnection::establish(&db_url)
            .expect("Establishing MySQL connection")),

            #[cfg(feature = "postgres")]
        DbBackend::Pg => DbConnection::Pg(
            PgConnection::establish(&db_url)
            .expect("Establishing PostgreSQL connection")),

            #[cfg(feature = "sqlite")]
        DbBackend::Sqlite => DbConnection::Sqlite(
            SqliteConnection::establish(&db_url)
            .expect("Establishing SQLite connection"))
    }
}

pub type DbConnectionInfo = (DbBackend, String);

pub fn establish_connection_info(db_connection_info: &DbConnectionInfo) -> DbConnection {
    establish_connection(db_connection_info.0, &db_connection_info.1)
}
