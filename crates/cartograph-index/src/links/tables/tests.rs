use proptest::prelude::*;

use super::*;

/// One line per fact: `kind name source how op line`.
fn show(path: &str, src: &str) -> String {
    let mut out = String::new();
    for f in scan(path, src.as_bytes()) {
        let a = |k: &str| match f.attrs.get(k) {
            Some(cartograph_facts::schema::AttrValue::Str(s)) => s.as_str().to_owned(),
            _ => "-".into(),
        };
        out.push_str(&format!(
            "{} {} {} {} {} line {}\n",
            f.kind.as_str(),
            a("name"),
            a("source"),
            a("how"),
            a("op"),
            f.span.0[0]
        ));
    }
    out
}

#[test]
fn sql_migrations_golden() {
    let sql = "-- orders\nCREATE TABLE public.orders (id int primary key, total numeric);\n\
               ALTER TABLE orders ADD COLUMN note text;\n\
               CREATE INDEX orders_total ON orders (total);\n\
               DROP TABLE IF EXISTS \"legacy\";\n\
               CREATE FUNCTION f() RETURNS int AS $$ BEGIN RETURN 1; END $$ LANGUAGE plpgsql;\n\
               CREATE TABLE `items` (id int);\n";
    insta::assert_snapshot!(show("db/migrations/001.sql", sql));
}

#[test]
fn prisma_golden() {
    let src =
        "model User {\n  id Int @id\n}\n\nmodel Order {\n  id Int @id\n  @@map(\"orders\")\n}\n";
    insta::assert_snapshot!(show("prisma/schema.prisma", src));
}

#[test]
fn typeorm_golden() {
    let src = "@Entity('orders')\nexport class Order {}\n\n@Entity()\n@Index(['a'])\nexport class UserProfile {}\n\n@Entity({ name: \"carts\" })\nexport default class Cart {}\n";
    insta::assert_snapshot!(show("src/entities.ts", src));
}

#[test]
fn sqlalchemy_golden() {
    let src = "class Order(Base):\n    __tablename__ = \"orders\"\n    id = Column(Integer)\n";
    insta::assert_snapshot!(show("app/models.py", src));
}

#[test]
fn django_golden() {
    let src = "class Order(models.Model):\n    total = models.IntegerField()\n\nclass Item(models.Model):\n    class Meta:\n        db_table = 'line_items'\n";
    insta::assert_snapshot!(show("shop/models.py", src));
}

#[test]
fn gorm_golden() {
    let src = "type Order struct {\n\tgorm.Model\n\tTotal int\n}\n\ntype Category struct {\n\tID uint `gorm:\"primaryKey\"`\n}\n\ntype Cart struct {\n\tgorm.Model\n}\n\nfunc (Cart) TableName() string { return \"shopping_carts\" }\n";
    insta::assert_snapshot!(show("models/models.go", src));
}

#[test]
fn eloquent_golden() {
    let src = "<?php\nclass Order extends Model\n{\n}\n";
    let named =
        "<?php\nclass Invoice extends Model\n{\n    protected $table = 'billing_invoices';\n}\n";
    insta::assert_snapshot!(format!(
        "{}{}",
        show("app/Order.php", src),
        show("app/Invoice.php", named)
    ));
}

#[test]
fn sql_literals_golden() {
    let src = "const a = \"SELECT o.id FROM orders o JOIN users u ON u.id = o.user_id\";\n\
               db.query(`INSERT INTO audit_log (msg) SELECT name FROM users WHERE id = ${id}`);\n\
               db.exec('update carts set n = 1 where id = $1');\n\
               const w = \"WITH t AS (SELECT 1 FROM base) SELECT * FROM t\";\n\
               const prose = \"select your plan\";\n\
               const cmd = \"DELETE FROM sessions WHERE expired\";\n\
               const x = \"hello SELECT * FROM nothing\";\n";
    insta::assert_snapshot!(show("src/repo.ts", src));
}

#[test]
fn a_literal_over_the_cap_is_not_parsed() {
    let pad = "x".repeat(MAX_LITERAL);
    let src = format!("const a = \"SELECT * FROM orders WHERE note = '{pad}'\";\n");
    assert_eq!(show("a.ts", &src), "");
    let ok = "const a = \"SELECT * FROM orders\";\n";
    assert!(show("a.ts", ok).contains("db.read orders"));
}

#[test]
fn deep_nesting_is_cut_by_the_recursion_limit() {
    let deep = format!("SELECT {}1{} FROM t", "(".repeat(5000), ")".repeat(5000));
    let src = format!("const a = \"{deep}\";");
    assert_eq!(show("a.ts", &src), "");
}

#[test]
fn a_file_that_is_not_utf8_has_no_facts() {
    assert!(scan("a.sql", &[0xff, 0xfe, b'C']).is_empty());
}

proptest! {
    // The SQL-literal path never panics and never reports a name that isn't an identifier.
    #[test]
    fn literals_never_panic(body in "\\PC{0,300}", q in "[\"'`]", kw in "(select|SELECT|insert into|update|delete from|with)") {
        let src = format!("x = {q}{kw} {body}{q};\n{q}{kw} {body}");
        for f in scan("a.ts", src.as_bytes()) {
            if let Some(cartograph_facts::schema::AttrValue::Str(n)) = f.attrs.get("name") {
                prop_assert!(clean(n.as_str()).as_deref() == Some(n.as_str()));
            }
        }
    }

    #[test]
    fn any_text_scans_in_every_language(text in "\\PC{0,200}") {
        for p in ["a.sql", "a.prisma", "a.ts", "a.py", "a.go", "a.php"] {
            let _ = scan(p, text.as_bytes());
        }
    }

    #[test]
    fn merge_and_intersect_agree_with_a_set_of_steps(
        a in prop::collection::vec((0i64..20, prop::option::of(0i64..25)), 0..5),
        b in prop::collection::vec((0i64..20, prop::option::of(0i64..25)), 0..5),
    ) {
        let live = |v: &[Span], s: i64| v.iter().any(|(f, t)| *f <= s && t.is_none_or(|t| s < t));
        let (ma, mb) = (merge(a.clone()), merge(b.clone()));
        let both = intersect(&ma, &mb);
        for s in 0..30 {
            prop_assert_eq!(live(&ma, s), live(&a, s));
            prop_assert_eq!(live(&both, s), live(&a, s) && live(&b, s));
        }
    }
}

#[test]
fn a_use_resolves_to_a_definition_by_how_sure_the_name_is() {
    let mut d = Defs::default();
    d.add("orders", 2, false, (0, None));
    d.add("users", 3, true, (0, None));
    d.add("billing_invoices", 4, false, (0, None));
    d.finish();
    assert_eq!(
        d.resolve("orders", 1).map(|x| (x.0.1, x.1)),
        Some((2, Match::Defined))
    );
    assert_eq!(
        d.resolve("users", 1).map(|x| (x.0.1, x.1)),
        Some((3, Match::Convention))
    );
    assert_eq!(
        d.resolve("Orders", 1).map(|x| (x.0.1, x.1)),
        Some((2, Match::NameOnly))
    );
    assert_eq!(
        d.resolve("BillingInvoice", 1).map(|x| (x.0.1, x.1)),
        Some((4, Match::NameOnly))
    );
    assert_eq!(d.resolve("carts", 1), None);
}

#[test]
fn a_model_beats_a_derived_name_and_your_own_service_beats_another() {
    let mut d = Defs::default();
    d.add("t", 5, true, (0, None));
    d.add("t", 6, false, (0, None));
    d.add("u", 7, false, (0, None));
    d.add("u", 8, false, (0, None));
    d.finish();
    assert_eq!(d.resolve("t", 5).map(|x| x.0.1), Some(6));
    assert_eq!(d.resolve("u", 8).map(|x| x.0.1), Some(8));
    assert_eq!(d.resolve("u", 1).map(|x| x.0.1), Some(7));
}
