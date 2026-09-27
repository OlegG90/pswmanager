import base64
import json
import os
import sys
import tempfile
import unittest
import zlib

from pykeepass import PyKeePass

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import sic2kdbx  # noqa: E402

PNG = b"\x89PNG\r\n\x1a\n" + b"\x00" * 16
ZIP = b"PK\x03\x04" + b"\x00" * 16
TEXT = "Привіт, notes\n".encode("utf-8")

XML = f"""<?xml version="1.0" encoding="utf-8"?>
<database>
  <card title="Router" id="1" symbol="server" color="red" star="true"
        time_stamp="1700000000000" first_stamp="1500000000000">
    <field name="Host IP" type="text" history="{{&quot;1500000000000&quot;:&quot;&quot;,&quot;1600000000000&quot;:&quot;10.0.0.1&quot;}}">10.0.0.2</field>
    <field name="Login" type="login">admin</field>
    <field name="Password" type="password" history="{{&quot;1500000000000&quot;:&quot;&quot;,&quot;1600000000000&quot;:&quot;old-pass&quot;}}">new-pass</field>
    <field name="PIN" type="pin">1234</field>
    <field name="Empty" type="text" />
    <field name="Remarks" type="text">line 1
line 2</field>
    <field name="2FA" type="one_time_password">JBSW Y3DP</field>
    <notes>free notes</notes>
    <label_id>10</label_id>
    <label_id>20</label_id>
    <image>{base64.b64encode(PNG).decode()}</image>
    <file name="sign.zip">{base64.b64encode(ZIP).decode()}</file>
    <file name="notes.txt">{base64.b64encode(zlib.compress(TEXT)).decode()}</file>
  </card>
  <card title="Mail" id="2" time_stamp="1700000000000">
    <field name="User name" type="login" />
    <field name="E-Mail" type="email">me@example.com</field>
    <field name="Website" type="website">example.com</field>
    <field name="Password" type="password">p</field>
  </card>
  <card title="Gone" id="3" deleted="true" time_stamp="1700000000000">
    <field name="Password" type="password">x</field>
    <label_id>20</label_id>
  </card>
  <card title="Car template" id="4" template="true">
    <field name="VIN" type="text" />
  </card>
  <card title='Say "hi"' id="5">
    <field name='Code "A"' type="text">42</field>
    <image>not base64!</image>
    <label_id>30</label_id>
  </card>
  <label name="NET" id="10" />
  <label name="Work; Home, X" id="30" />
  <label name="Old" id="20" />
</database>
"""


class ConvertTest(unittest.TestCase):
    """Each conversion runs Argon2 twice (save + open), so the default one is shared."""

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.xml = os.path.join(cls.tmp.name, "in.xml")
        with open(cls.xml, "w", encoding="utf-8") as f:
            f.write(XML)
        cls.default = cls.convert_with()

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    @classmethod
    def convert_with(cls, **kw):
        kdbx = os.path.join(cls.tmp.name, f"out{len(os.listdir(cls.tmp.name))}.kdbx")
        stats = sic2kdbx.convert(sic2kdbx.parse_sic(cls.xml), kdbx, "pw", **kw)
        return stats, PyKeePass(kdbx, "pw")

    def convert(self, **kw):
        return self.convert_with(**kw) if kw else self.default

    def entry(self, kp, title):
        return kp.find_entries(title=title, first=True)

    def test_standard_and_custom_fields(self):
        _, kp = self.convert()
        e = self.entry(kp, "Router")
        self.assertEqual((e.username, e.password, e.notes), ("admin", "new-pass", "free notes"))
        self.assertEqual(e.get_custom_property("Host IP"), "10.0.0.2")
        self.assertEqual(e.get_custom_property("Remarks"), "line 1\nline 2")
        self.assertTrue(e.is_custom_property_protected("PIN"))
        self.assertFalse(e.is_custom_property_protected("Host IP"))
        self.assertIn("Empty", e.custom_properties)
        self.assertEqual(e.otp, "otpauth://totp/Router?secret=JBSWY3DP")

    def test_groups_tags_attachments(self):
        _, kp = self.convert()
        e = self.entry(kp, "Router")
        self.assertEqual(e.group.name, "NET")
        self.assertEqual(e.tags, ["NET", "Old", "Favorite"])
        self.assertEqual({a.filename: a.data for a in e.attachments},
                         {"image.png": PNG, "sign.zip": ZIP, "notes.txt": TEXT})

    def test_unpack_leaves_what_is_not_zlib(self):
        self.assertEqual(sic2kdbx._unpack(zlib.compress(TEXT)), TEXT)
        for data in (PNG, ZIP, b"x\x9c broken", b""):
            self.assertEqual(sic2kdbx._unpack(data), data)

    def test_history_and_times(self):
        _, kp = self.convert()
        e = self.entry(kp, "Router")
        self.assertEqual([h.password for h in e.history], ["old-pass"])
        self.assertEqual(e.history[0].get_custom_property("Host IP"), "10.0.0.1")
        self.assertEqual(e.ctime.year, 2017)
        self.assertEqual(int(e.mtime.timestamp()), 1700000000)

    def test_drop_empty(self):
        _, kp = self.convert(keep_empty=False)
        self.assertNotIn("Empty", self.entry(kp, "Router").custom_properties)
        self.assertIn("VIN", self.entry(kp, "Car template").custom_properties)

    def test_quotes_labels_and_bad_attachment(self):
        _, kp = self.convert()
        # find_entries builds XPath from the title and breaks on quotes
        e = next(x for x in kp.entries if x.title == 'Say "hi"')
        fields = {s.findtext("Key"): s.findtext("Value") for s in e._element.findall("String")}
        self.assertEqual(fields['Code "A"'], "42")
        self.assertEqual(e.group.name, "Work; Home, X")
        self.assertEqual(e.tags, ["Work Home X"])
        self.assertEqual(e.attachments, [])

    def test_email_fallback_for_username(self):
        _, kp = self.convert()
        e = self.entry(kp, "Mail")
        self.assertEqual((e.username, e.url), ("me@example.com", "example.com"))
        self.assertIsNone(e.get_custom_property("E-Mail"))
        self.assertEqual(e.group, kp.root_group)

    def test_custom_data_layout(self):
        _, kp = self.convert()
        e = self.entry(kp, "Router")
        raw = e._element.find("CustomData/Item[Key='SafeInCloud']/Value").text
        data = json.loads(raw)
        self.assertEqual(data["symbol"], "server")
        self.assertEqual(data["fields"][0], {"name": "Host IP", "type": "text", "key": "Host IP"})

    def test_elements_are_in_keepass_order(self):
        _, kp = self.convert()
        for tag, order in (("Entry", sic2kdbx.ENTRY_ORDER), ("Group", sic2kdbx.GROUP_ORDER)):
            for element in kp.tree.iter(tag):
                tags = [c.tag for c in element]
                ranks = [order.index(t) for t in tags if t in order]
                self.assertEqual(ranks, sorted(ranks), tags)

    def test_deleted_and_templates(self):
        stats, kp = self.convert()
        self.assertEqual(self.entry(kp, "Gone").group.name, kp.recyclebin_group.name)
        tpl = self.entry(kp, "Car template")
        self.assertEqual(tpl.group.name, "Templates")
        marked = kp.tree.find("Meta/EntryTemplatesGroup").text
        self.assertEqual(marked, base64.b64encode(tpl.group.uuid.bytes).decode())
        self.assertIn("VIN", tpl.custom_properties)  # empty value is kept in templates
        self.assertEqual((stats["entries"], stats["deleted"]), (5, 1))

        stats, kp = self.convert(skip_deleted=True)
        self.assertIsNone(self.entry(kp, "Gone"))
        self.assertEqual(stats["skipped"], 1)


if __name__ == "__main__":
    unittest.main()
