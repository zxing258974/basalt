use super::test_support::*;
use super::*;

const M: Dialect = Dialect::Mysql;

fn users() -> TableDesign {
    let mut name = not_null(col("name", "varchar(50)"));
    name.default = Some("''".into());
    name.comment = Some("姓名".into());
    name.collation = Some("utf8mb4_bin".into());
    let mut updated = col("updated_at", "timestamp");
    updated.default = Some("CURRENT_TIMESTAMP".into());
    updated.on_update = Some("CURRENT_TIMESTAMP".into());
    TableDesign {
        primary_key: vec![cid("id")],
        indexes: vec![index("idx_name", &["name"])],
        ..design(
            M,
            "users",
            vec![
                ColumnDef {
                    auto_increment: true,
                    ..not_null(col("id", "int unsigned"))
                },
                name,
                col("email", "varchar(100)"),
                updated,
            ],
        )
    }
}

#[test]
fn create_table_lists_columns_keys_indexes_and_options() {
    let mut target = users();
    target.columns.push(col("team_id", "int"));
    let mut email = index("uk_email", &["email"]);
    email.unique = true;
    let mut prefix = index("idx_prefix", &["name DESC"]);
    prefix.columns[0].length = Some(10);
    let mut fulltext = index("ft_name", &["name"]);
    fulltext.method = Some("FULLTEXT".into());
    target.indexes = vec![email, prefix, fulltext];
    let mut team = fk("fk_team", &["team_id"], "teams", &["id"]);
    team.on_delete = FkAction::Cascade;
    target.foreign_keys = vec![team];
    target.options = TableOptions {
        engine: Some("InnoDB".into()),
        charset: Some("utf8mb4".into()),
        collation: Some("utf8mb4_general_ci".into()),
        comment: Some("用户".into()),
        ..TableOptions::default()
    };

    let plan = plan_for(M, None, &target);

    assert_eq!(plan.mode, ExecMode::Autocommit);
    assert_eq!(
        sqls(&plan),
        ["CREATE TABLE `shop`.`users` (\n  \
          `id` int unsigned NOT NULL AUTO_INCREMENT,\n  \
          `name` varchar(50) COLLATE utf8mb4_bin NOT NULL DEFAULT '' COMMENT '姓名',\n  \
          `email` varchar(100) NULL,\n  \
          `updated_at` timestamp NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,\n  \
          `team_id` int NULL,\n  \
          PRIMARY KEY (`id`),\n  \
          UNIQUE INDEX `uk_email` (`email`),\n  \
          INDEX `idx_prefix` (`name`(10) DESC),\n  \
          FULLTEXT INDEX `ft_name` (`name`),\n  \
          CONSTRAINT `fk_team` FOREIGN KEY (`team_id`) REFERENCES `shop`.`teams` (`id`) ON DELETE CASCADE\n\
          ) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_general_ci COMMENT = '用户'"]
    );
}

#[test]
fn generated_and_hash_index_render_in_mysql_syntax() {
    let mut total = col("total", "decimal(10,2)");
    total.generated = Some(Generated {
        expression: "price * qty".into(),
        stored: false,
    });
    let mut hash = index("idx_total", &["total"]);
    hash.method = Some("HASH".into());
    let target = TableDesign {
        indexes: vec![hash],
        ..design(
            M,
            "lines",
            vec![col("price", "decimal(10,2)"), col("qty", "int"), total],
        )
    };
    let plan = plan_for(M, None, &target);
    let sql = sqls(&plan)[0];
    assert!(sql.contains("`total` decimal(10,2) GENERATED ALWAYS AS (price * qty) VIRTUAL NULL"));
    assert!(sql.contains("INDEX `idx_total` (`total`) USING HASH"));
}

#[test]
fn rename_is_a_change_column_not_drop_and_add() {
    let original = users();
    let mut target = users();
    target.columns[1].name = "full_name".into();

    let plan = plan_for(M, Some(&original), &target);

    assert_eq!(
        sqls(&plan),
        ["ALTER TABLE `shop`.`users`\n  \
          CHANGE COLUMN `name` `full_name` varchar(50) COLLATE utf8mb4_bin NOT NULL DEFAULT '' COMMENT '姓名'"]
    );
}

#[test]
fn modify_column_restates_every_attribute() {
    let original = users();
    let mut target = users();
    target.columns[3].nullable = false;

    assert_eq!(
        sqls(&plan_for(M, Some(&original), &target)),
        ["ALTER TABLE `shop`.`users`\n  \
          MODIFY COLUMN `updated_at` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP"]
    );
}

#[test]
fn unchanged_design_produces_no_statements() {
    let plan = plan_for(M, Some(&users()), &users());
    assert!(plan.statements.is_empty());
}

#[test]
fn added_columns_get_positions_unless_appended() {
    let original = users();
    let mut target = users();
    target.columns.insert(0, new_col("tenant", "int"));
    target.columns.insert(3, new_col("phone", "varchar(20)"));
    target.columns.push(new_col("note", "text"));
    target.columns.retain(|c| c.name != "email");

    assert_eq!(
        sqls(&plan_for(M, Some(&original), &target)),
        ["ALTER TABLE `shop`.`users`\n  \
          DROP COLUMN `email`,\n  \
          ADD COLUMN `tenant` int NULL FIRST,\n  \
          ADD COLUMN `phone` varchar(20) NULL AFTER `name`,\n  \
          ADD COLUMN `note` text NULL"]
    );
}

#[test]
fn reorder_moves_only_columns_out_of_place() {
    let original = users();
    let mut target = users();
    // id, name, email, updated_at → id, updated_at, name, email:只需挪 updated_at。
    let updated = target.columns.remove(3);
    target.columns.insert(1, updated);

    assert_eq!(
        sqls(&plan_for(M, Some(&original), &target)),
        ["ALTER TABLE `shop`.`users`\n  \
          MODIFY COLUMN `updated_at` timestamp NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP AFTER `id`"]
    );
}

#[test]
fn position_refers_to_the_new_name_of_a_renamed_neighbour() {
    let original = users();
    let mut target = users();
    target.columns[0].name = "user_id".into();
    target.columns.insert(1, new_col("tenant", "int"));

    let plan = plan_for(M, Some(&original), &target);
    assert!(sqls(&plan)[0].ends_with("ADD COLUMN `tenant` int NULL AFTER `user_id`"));
}

#[test]
fn primary_key_change_drops_and_adds_in_one_statement() {
    let original = users();
    let mut target = users();
    target.primary_key = vec![cid("id"), cid("email")];

    assert_eq!(
        sqls(&plan_for(M, Some(&original), &target)),
        ["ALTER TABLE `shop`.`users`\n  DROP PRIMARY KEY,\n  ADD PRIMARY KEY (`id`, `email`)"]
    );
}

#[test]
fn index_follows_renamed_column_without_rebuild() {
    let original = users();
    let mut target = users();
    target.columns[1].name = "full_name".into();
    let plan = plan_for(M, Some(&original), &target);
    assert!(!sqls(&plan)[0].contains("INDEX"));
}

#[test]
fn index_changes_drop_then_add() {
    let original = users();
    let mut target = users();
    target.indexes[0].name = "idx_full".into();
    target.indexes.push(new_index("idx_email", &["email DESC"]));

    assert_eq!(
        sqls(&plan_for(M, Some(&original), &target)),
        ["ALTER TABLE `shop`.`users`\n  \
          DROP INDEX `idx_name`,\n  \
          ADD INDEX `idx_full` (`name`),\n  \
          ADD INDEX `idx_email` (`email` DESC)"]
    );
}

#[test]
fn foreign_keys_are_dropped_in_a_statement_of_their_own() {
    let mut original = users();
    original.columns.push(col("team_id", "int"));
    original.foreign_keys = vec![
        fk("fk_team", &["team_id"], "teams", &["id"]),
        fk("fk_gone", &["team_id"], "old_teams", &["id"]),
    ];
    let mut target = original.clone();
    target.foreign_keys.remove(1);
    target.foreign_keys[0].on_delete = FkAction::SetNull;
    target.foreign_keys[0].ref_schema = Some("hr".into());
    target
        .foreign_keys
        .push(new_fk("", &["id"], "audit", &["user_id"]));

    assert_eq!(
        sqls(&plan_for(M, Some(&original), &target)),
        [
            "ALTER TABLE `shop`.`users`\n  DROP FOREIGN KEY `fk_team`,\n  DROP FOREIGN KEY `fk_gone`",
            "ALTER TABLE `shop`.`users`\n  \
             ADD CONSTRAINT `fk_team` FOREIGN KEY (`team_id`) REFERENCES `hr`.`teams` (`id`) ON DELETE SET NULL,\n  \
             ADD FOREIGN KEY (`id`) REFERENCES `shop`.`audit` (`user_id`)",
        ]
    );
}

#[test]
fn table_options_and_rename_come_last() {
    let mut original = users();
    original.options = TableOptions {
        engine: Some("MyISAM".into()),
        charset: Some("latin1".into()),
        collation: Some("latin1_swedish_ci".into()),
        comment: Some("旧".into()),
        ..TableOptions::default()
    };
    let mut target = original.clone();
    target.table.table = "members".into();
    target.options.engine = Some("InnoDB".into());
    target.options.charset = Some("utf8mb4".into());
    target.options.collation = Some("utf8mb4_bin".into());
    target.options.comment = Some("会员 'VIP' \\ 备注".into());

    assert_eq!(
        sqls(&plan_for(M, Some(&original), &target)),
        ["ALTER TABLE `shop`.`users`\n  \
          ENGINE = InnoDB,\n  \
          DEFAULT CHARACTER SET = utf8mb4 COLLATE = utf8mb4_bin,\n  \
          COMMENT = '会员 ''VIP'' \\\\ 备注',\n  \
          RENAME TO `shop`.`members`"]
    );
}

#[test]
fn backslashes_stay_literal_under_no_backslash_escapes() {
    let mut target = users();
    target.options.comment = Some(r"a\b".into());
    let context = DdlContext {
        backslash_escapes: false,
        ..DdlContext::new(M)
    };
    let plan = plan(&context, None, &target).unwrap();
    assert!(plan.statements[0].sql.ends_with(r"COMMENT = 'a\b'"));
}

#[test]
fn removing_comment_writes_empty_comment() {
    let original = users();
    let mut target = users();
    target.columns[1].comment = None;
    let plan = plan_for(M, Some(&original), &target);
    assert!(sqls(&plan)[0].ends_with("NOT NULL DEFAULT ''"));

    let mut with_comment = users();
    with_comment.options.comment = Some("x".into());
    let plan = plan_for(M, Some(&with_comment), &users());
    assert!(sqls(&plan)[0].ends_with("COMMENT = ''"));
}

#[test]
fn stale_column_id_is_rejected() {
    let original = users();
    let mut target = users();
    target.columns.push(col("ghost", "int"));
    let message = plan_error(M, Some(&original), &target);
    assert!(message.contains("ghost"), "{message}");
    assert!(message.contains("重新读取"), "{message}");
}

#[test]
fn moving_a_foreign_key_drops_its_same_named_support_index() {
    let mut original = users();
    original.columns.push(col("team_id", "int"));
    original.columns.push(col("group_id", "int"));
    original.foreign_keys = vec![fk("fk_team", &["team_id"], "teams", &["id"])];
    original.indexes.push(index("fk_team", &["team_id"]));
    let mut target = original.clone();
    target.foreign_keys[0].columns = vec![cid("group_id")];

    let statements = plan_for(M, Some(&original), &target);
    assert_eq!(
        sqls(&statements),
        [
            "ALTER TABLE `shop`.`users`\n  DROP FOREIGN KEY `fk_team`",
            "ALTER TABLE `shop`.`users`\n  \
             DROP INDEX `fk_team`,\n  \
             ADD CONSTRAINT `fk_team` FOREIGN KEY (`group_id`) REFERENCES `shop`.`teams` (`id`)",
        ]
    );

    // 已经有索引能撑起新外键,或同名索引不是当初自动建的,就不动它。
    let mut covered = target.clone();
    covered.indexes.push(new_index("idx_group", &["group_id"]));
    assert!(!sqls(&plan_for(M, Some(&original), &covered))[1].contains("DROP INDEX `fk_team`"));
    let mut own = original.clone();
    own.indexes[1] = index("fk_team", &["team_id", "id"]);
    let mut own_target = own.clone();
    own_target.foreign_keys[0].columns = vec![cid("group_id")];
    assert!(!sqls(&plan_for(M, Some(&own), &own_target))[1].contains("DROP INDEX"));
}
