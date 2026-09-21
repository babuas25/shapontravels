"""All configured groups are exported; raw catalog anomalies stay visible."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('airport_audit', Path(__file__).with_name('audit-airport-mapping.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class MappingAuditTests(unittest.TestCase):
    def setUp(self):
        self.rows = [{'iata': c, 'city': 'Metro', 'iso': 'SG', 'status': 1, 'type': 'airport'}
                     for c in ['SIN', 'XSP']]
        self.groups = {'SIN_CITY': {'city': 'Singapore', 'countryCode': 'SG', 'airports': ['SIN', 'XSP']}}

    def test_all_groups_export_without_reviews_including_single_airports(self):
        self.groups['DAC_CITY'] = {'city': 'Dhaka', 'countryCode': 'BD', 'airports': ['DAC']}
        configured, report = module.audit(self.rows, self.groups, {})
        self.assertEqual([g['id'] for g in configured], ['DAC_CITY', 'SIN_CITY'])
        self.assertEqual(report['enabledGroups'], ['DAC_CITY', 'SIN_CITY'])
        self.assertEqual(report['missingGroupAirports'], ['DAC'])

    def test_conflicting_raw_records_are_reported_without_overriding_groups(self):
        rows = self.rows + [{**self.rows[0], 'city': 'Other', 'iso': 'CN'}]
        output = module.audit(rows, self.groups, {})
        self.assertEqual(output, module.audit(list(reversed(rows)), self.groups, {}))
        self.assertEqual(output[0][0]['country'], 'SG')
        self.assertIn('SIN', output[1]['conflictingLocations'])
        self.assertEqual(output[1]['groupCountryMismatches']['SIN_CITY/SIN'], ['CN', 'SG'])

    def test_overlap_preserves_both_groups(self):
        self.groups['OTHER'] = {'city': 'Other metro', 'countryCode': 'SG', 'airports': ['XSP', 'XXX']}
        configured, report = module.audit(self.rows, self.groups, {})
        self.assertEqual(len(configured), 2)
        self.assertEqual(report['overlappingGroups'], {'XSP': ['OTHER', 'SIN_CITY']})
        self.groups['OTHER']['countryCode'] = 'MY'
        with self.assertRaisesRegex(ValueError, 'conflicting group countries'):
            module.audit(self.rows, self.groups, {})

    def test_inactive_or_non_airport_raw_rows_do_not_disable_configured_members(self):
        for row in [{**self.rows[1], 'status': 0}, {**self.rows[1], 'type': 'city'}]:
            configured, report = module.audit([self.rows[0], row], self.groups, {})
            self.assertEqual(configured[0]['airports'], ['SIN', 'XSP'])
            self.assertTrue(report['missingGroupAirports'] or report['nonAirportGroupMembers'])

    def test_editing_group_never_requires_review_update(self):
        review = {'SIN_CITY': {'countryCode': 'SG', 'airports': ['SIN', 'XSP']}}
        self.groups['SIN_CITY']['airports'].append('XXX')
        configured, report = module.audit(self.rows, self.groups, review)
        self.assertEqual(configured[0]['airports'], ['SIN', 'XSP', 'XXX'])
        self.assertEqual(report['staleReviews'], ['SIN_CITY'])

    def test_invalid_configuration_is_rejected(self):
        for patch in [{'airports': []}, {'airports': ['SIN', 'SIN']}, {'airports': ['sin']},
                      {'airports': ['SIN_CITY']}, {'countryCode': 'SGP'}, {'city': ''}]:
            groups = copy.deepcopy(self.groups)
            groups['SIN_CITY'].update(patch)
            with self.assertRaises(ValueError):
                module.audit(self.rows, groups, {})


if __name__ == '__main__':
    unittest.main()
